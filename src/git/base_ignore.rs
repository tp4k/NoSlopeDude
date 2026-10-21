//! The base-ignore filter for `check --worktree`: an untracked worktree
//! entry is left out only when the base snapshot's `.gitignore` rules ignore
//! it AND the candidate's own on-disk `.gitignore` files also ignore it. The
//! candidate side can therefore only remove a skip, never add one.
//! `.git/info/exclude`, `core.excludesFile` and `core.ignoreCase` are never
//! consulted; matching is case-sensitive and byte-wise (git's wildmatch with
//! `WM_PATHNAME`, plus `dir.c`'s line parsing).
//!
//! A `.gitignore` that cannot be used on one side (over `SOURCE_CEILING_BYTES`,
//! an on-disk read failure, or a line too intricate for the matcher's stack
//! bound) makes that side ignore nothing at or below its directory: a failure
//! may expose more files, never hide more than git would.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read as _};
use std::path::PathBuf;

use git2::Repository;

use super::snapshot::{repo_path_to_fs, CommitSnapshot, EntryKind, SOURCE_CEILING_BYTES};
use super::wildmatch::wildmatch;
use super::GitError;

const GITIGNORE_NAME: &[u8] = b".gitignore";
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// A line with more star runs than this makes its file unavailable: the
/// matcher recurses once per run, so the bound keeps the stack bounded.
const MAX_STAR_RUNS_PER_LINE: usize = 1_000;

#[derive(Clone, Copy)]
enum Side {
    Base,
    Candidate,
}

/// One usable line of a `.gitignore`.
struct Rule {
    negation: bool,
    directory_only: bool,
    /// No `/` after the trailing one is stripped: matched against the last
    /// path component at any depth.
    basename: bool,
    /// The pattern without `!`, the trailing `/` and (for an anchored
    /// pattern) the leading `/`.
    pattern: Vec<u8>,
}

/// What one directory's `.gitignore` offers on one side.
enum Load {
    /// No file, or a symlink or other non-regular entry (never followed).
    Absent,
    Rules(Vec<Rule>),
    /// A file that exists but cannot be used.
    Unavailable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Decision {
    Ignore,
    Include,
    NoRule,
}

/// Per-directory rules for both sides, loaded lazily and cached.
pub(super) struct BaseIgnore<'a> {
    repo: &'a Repository,
    base: &'a CommitSnapshot,
    workdir: PathBuf,
    /// Directory (raw bytes, `""` for the root) to its `.gitignore` entry in
    /// the base snapshot.
    base_files: HashMap<Vec<u8>, usize>,
    base_rules: HashMap<Vec<u8>, Load>,
    candidate_rules: HashMap<Vec<u8>, Load>,
    /// Directory to "ignored on this side, ancestors included".
    base_dirs: HashMap<Vec<u8>, bool>,
    candidate_dirs: HashMap<Vec<u8>, bool>,
}

impl<'a> BaseIgnore<'a> {
    pub(super) fn new(
        repo: &'a Repository,
        base: &'a CommitSnapshot,
        workdir: PathBuf,
    ) -> BaseIgnore<'a> {
        let mut base_files = HashMap::new();
        for (index, entry) in base.entries.iter().enumerate() {
            let bytes = entry.path.as_bytes();
            let (dir, name) = match bytes.iter().rposition(|byte| *byte == b'/') {
                Some(slash) => (&bytes[..slash], &bytes[slash + 1..]),
                None => (&bytes[..0], bytes),
            };
            if name == GITIGNORE_NAME {
                base_files.insert(dir.to_vec(), index);
            }
        }
        BaseIgnore {
            repo,
            base,
            workdir,
            base_files,
            base_rules: HashMap::new(),
            candidate_rules: HashMap::new(),
            base_dirs: HashMap::new(),
            candidate_dirs: HashMap::new(),
        }
    }

    /// Whether the untracked path `path` is left out of the snapshot: both
    /// sides ignore it. The base side is asked first, so the candidate's
    /// `.gitignore` files are read only where the base already ignores.
    pub(super) fn skips(&mut self, path: &[u8], is_dir: bool) -> Result<bool, GitError> {
        Ok(self.ignores(Side::Base, path, is_dir)?
            && self.ignores(Side::Candidate, path, is_dir)?)
    }

    /// Git's top-down rule: a path is ignored when any ancestor directory is
    /// ignored (a child cannot be re-included) or the path itself is. The
    /// walk is top-down, so each parent's verdict is memoized per side.
    fn ignores(&mut self, side: Side, path: &[u8], is_dir: bool) -> Result<bool, GitError> {
        let ignored = match path.iter().rposition(|byte| *byte == b'/') {
            Some(slash) => {
                self.dir_ignored(side, &path[..slash])? || self.node_ignored(side, path, is_dir)?
            }
            None => self.node_ignored(side, path, is_dir)?,
        };
        if is_dir {
            self.dir_memo(side).insert(path.to_vec(), ignored);
        }
        Ok(ignored)
    }

    /// Whether directory `dir` is ignored on `side`, ancestors included.
    fn dir_ignored(&mut self, side: Side, dir: &[u8]) -> Result<bool, GitError> {
        if let Some(ignored) = self.dir_memo(side).get(dir) {
            return Ok(*ignored);
        }
        self.ignores(side, dir, true)
    }

    fn dir_memo(&mut self, side: Side) -> &mut HashMap<Vec<u8>, bool> {
        match side {
            Side::Base => &mut self.base_dirs,
            Side::Candidate => &mut self.candidate_dirs,
        }
    }

    /// The verdict for `node` alone (its ancestors are not asked). The
    /// directories on its path are consulted root-first for an unavailable
    /// `.gitignore`, which makes this side ignore nothing here; otherwise the
    /// deepest `.gitignore` with a matching line decides and a whitelist line
    /// un-ignores. Every ancestor of `node` is a directory.
    fn node_ignored(
        &mut self,
        side: Side,
        node: &[u8],
        node_is_dir: bool,
    ) -> Result<bool, GitError> {
        let slashes: Vec<usize> = node
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'/').then_some(index))
            .collect();
        let mut dir_ends: Vec<Option<usize>> = vec![None];
        dir_ends.extend(slashes.into_iter().map(Some));
        for end in &dir_ends {
            let dir = end.map_or(&node[..0], |slash| &node[..slash]);
            self.ensure_loaded(side, dir)?;
            if matches!(self.rules(side).get(dir), Some(Load::Unavailable)) {
                return Ok(false);
            }
        }
        for end in dir_ends.iter().rev() {
            let (dir, relative) = match end {
                Some(slash) => (&node[..*slash], &node[slash + 1..]),
                None => (&node[..0], node),
            };
            let Some(Load::Rules(rules)) = self.rules(side).get(dir) else {
                continue;
            };
            match decide(rules, relative, node_is_dir) {
                Decision::Ignore => return Ok(true),
                Decision::Include => return Ok(false),
                Decision::NoRule => {}
            }
        }
        Ok(false)
    }

    fn rules(&self, side: Side) -> &HashMap<Vec<u8>, Load> {
        match side {
            Side::Base => &self.base_rules,
            Side::Candidate => &self.candidate_rules,
        }
    }

    /// Loads and caches the `.gitignore` of `dir` on `side`.
    fn ensure_loaded(&mut self, side: Side, dir: &[u8]) -> Result<(), GitError> {
        if self.rules(side).contains_key(dir) {
            return Ok(());
        }
        let load = match side {
            Side::Base => self.read_base(dir)?,
            Side::Candidate => self.read_candidate(dir),
        };
        match side {
            Side::Base => self.base_rules.insert(dir.to_vec(), load),
            Side::Candidate => self.candidate_rules.insert(dir.to_vec(), load),
        };
        Ok(())
    }

    /// The base `.gitignore` blob of `dir`: absent (or not a regular file),
    /// parsed rules, or unavailable when over the ceiling. A blob that cannot
    /// be read is an error.
    fn read_base(&self, dir: &[u8]) -> Result<Load, GitError> {
        let Some(index) = self.base_files.get(dir) else {
            return Ok(Load::Absent);
        };
        let entry = &self.base.entries[*index];
        if !matches!(entry.kind, EntryKind::Regular | EntryKind::Executable) {
            return Ok(Load::Absent);
        }
        if entry.size > SOURCE_CEILING_BYTES {
            return Ok(Load::Unavailable);
        }
        Ok(match self.base.read(self.repo, entry)? {
            Some(bytes) => load_rules(&bytes),
            None => Load::Unavailable,
        })
    }

    /// The on-disk `.gitignore` of `dir`: regular files only (never a
    /// symlink), bounded by the ceiling; an open or read failure makes the
    /// file unavailable.
    fn read_candidate(&self, dir: &[u8]) -> Load {
        let mut relative = dir.to_vec();
        if !relative.is_empty() {
            relative.push(b'/');
        }
        relative.extend_from_slice(GITIGNORE_NAME);
        let full_path = repo_path_to_fs(&self.workdir, &relative);
        match fs::symlink_metadata(&full_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Load::Absent,
            Err(err) if err.kind() == ErrorKind::NotFound => return Load::Absent,
            Err(_) => return Load::Unavailable,
        }
        let Ok(file) = fs::File::open(&full_path) else {
            return Load::Unavailable;
        };
        let mut buf = Vec::new();
        if file
            .take(SOURCE_CEILING_BYTES + 1)
            .read_to_end(&mut buf)
            .is_err()
            || buf.len() as u64 > SOURCE_CEILING_BYTES
        {
            return Load::Unavailable;
        }
        load_rules(&buf)
    }
}

/// The last rule of `rules` that matches `relative` decides.
fn decide(rules: &[Rule], relative: &[u8], is_dir: bool) -> Decision {
    for rule in rules.iter().rev() {
        if rule.matches(relative, is_dir) {
            return if rule.negation {
                Decision::Include
            } else {
                Decision::Ignore
            };
        }
    }
    Decision::NoRule
}

impl Rule {
    fn matches(&self, relative: &[u8], is_dir: bool) -> bool {
        if self.directory_only && !is_dir {
            return false;
        }
        if self.basename {
            let name = match relative.iter().rposition(|byte| *byte == b'/') {
                Some(slash) => &relative[slash + 1..],
                None => relative,
            };
            return wildmatch(&self.pattern, name);
        }
        wildmatch(&self.pattern, relative)
    }
}

/// The rules of a `.gitignore`, or `Unavailable` when a line is too intricate.
fn load_rules(bytes: &[u8]) -> Load {
    match parse_rules(bytes) {
        Some(rules) => Load::Rules(rules),
        None => Load::Unavailable,
    }
}

/// Git's `.gitignore` parsing. One leading UTF-8 BOM is skipped; lines split
/// on `\n`; one `\r` before the `\n` is dropped; a line is a C string, so it
/// is cut at its first NUL; an empty line and a line starting with `#` add
/// nothing; trailing unescaped spaces are trimmed (U+0020 only). `None` when a
/// line has more star runs than `MAX_STAR_RUNS_PER_LINE`.
fn parse_rules(bytes: &[u8]) -> Option<Vec<Rule>> {
    let bytes = bytes.strip_prefix(UTF8_BOM.as_slice()).unwrap_or(bytes);
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut rules = Vec::new();
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = line.split(|byte| *byte == 0).next().unwrap_or(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        if let Some(rule) = parse_line(trim_unescaped_trailing_spaces(line))? {
            rules.push(rule);
        }
    }
    Some(rules)
}

/// One trimmed line as a rule: `Some(None)` when it can never match, `None`
/// when it is too intricate.
fn parse_line(line: &[u8]) -> Option<Option<Rule>> {
    let negation = line.first() == Some(&b'!');
    let mut body = &line[usize::from(negation)..];
    let directory_only = body.last() == Some(&b'/');
    if directory_only {
        body = &body[..body.len() - 1];
    }
    if body.is_empty() {
        return Some(None);
    }
    if star_runs(body) > MAX_STAR_RUNS_PER_LINE {
        return None;
    }
    let basename = !body.contains(&b'/');
    if !basename && body[0] == b'/' {
        body = &body[1..];
    }
    Some(Some(Rule {
        negation,
        directory_only,
        basename,
        pattern: body.to_vec(),
    }))
}

fn star_runs(pattern: &[u8]) -> usize {
    pattern
        .iter()
        .enumerate()
        .filter(|(index, byte)| **byte == b'*' && (*index == 0 || pattern[index - 1] != b'*'))
        .count()
}

/// Git's `trim_trailing_spaces`: drop trailing U+0020 unless a backslash
/// escapes the space.
fn trim_unescaped_trailing_spaces(line: &[u8]) -> &[u8] {
    let mut last_space = None;
    let mut index = 0;
    while index < line.len() {
        match line[index] {
            b' ' => {
                last_space.get_or_insert(index);
            }
            b'\\' => {
                if index + 1 >= line.len() {
                    return line;
                }
                last_space = None;
                index += 1;
            }
            _ => last_space = None,
        }
        index += 1;
    }
    last_space.map_or(line, |index| &line[..index])
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::{fs, parse_rules, BaseIgnore, CommitSnapshot, Load, Repository};

    fn chained(runs: usize) -> Vec<u8> {
        let mut line = b"**/".repeat(runs);
        line.extend_from_slice(b"x\n");
        line
    }

    fn numbered(count: usize, make: impl Fn(usize) -> String) -> Vec<u8> {
        (0..count)
            .flat_map(|index| make(index).into_bytes())
            .collect()
    }

    #[test]
    fn test_too_many_double_star_runs_in_one_file_make_it_unavailable() {
        let two_long = [chained(600), chained(600)].concat();
        assert!(parse_rules(&two_long).is_none(), "1198 extra runs");

        let one_long = [chained(1_000), b"*.log\ntarget/\n".to_vec()].concat();
        assert!(parse_rules(&one_long).is_some(), "999 extra runs");

        let at_budget = numbered(1_000, |i| format!("**/d{i}/**\n"));
        assert!(parse_rules(&at_budget).is_some(), "1000 extra runs");

        let single_runs = numbered(1_500, |i| format!("**/x{i}\n"));
        assert!(parse_rules(&single_runs).is_some(), "0 extra runs");

        let plain_stars = numbered(1_500, |i| format!("*a*b*c{i}\n"));
        assert!(parse_rules(&plain_stars).is_some(), "0 special runs");
    }

    #[test]
    fn test_an_unopenable_candidate_gitignore_is_unavailable_not_absent() {
        let dir = tempfile::TempDir::new().expect("create the temp dir");
        let repo = Repository::init(dir.path()).expect("init the repository");
        let base = CommitSnapshot {
            entries: Vec::new(),
        };
        fs::create_dir(dir.path().join("p")).expect("create p");
        let locked = dir.path().join("p/.gitignore");
        fs::write(&locked, b"!X.java\n").expect("write the file");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock the file");
        let readable_anyway = fs::File::open(&locked).is_ok();
        let ignore = BaseIgnore::new(&repo, &base, dir.path().to_path_buf());

        let load = ignore.read_candidate(b"p");

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).expect("unlock the file");
        if readable_anyway {
            return;
        }
        assert!(matches!(load, Load::Unavailable));
    }
}

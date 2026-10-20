//! The base-ignore filter for `check --worktree`: an untracked worktree
//! entry is left out only when the base snapshot's `.gitignore` rules
//! ignore it AND the candidate's own on-disk `.gitignore` files also ignore
//! it. The candidate side can therefore only remove a skip, never add one.
//! `.git/info/exclude`, `core.excludesFile` and `core.ignoreCase` are never
//! consulted; matching is case-sensitive.

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use git2::Repository;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

use super::snapshot::{repo_path_to_fs, CommitSnapshot, SOURCE_CEILING_BYTES};
use super::GitError;

const GITIGNORE_NAME: &[u8] = b".gitignore";
const UTF8_BOM: char = '\u{feff}';
/// `ignore`'s builder strips a leading `./` and, for a root of `.`, strips
/// nothing else, so paths handed to `matched` stay relative to their own
/// `.gitignore` directory.
const MATCHER_ROOT: &str = ".";

#[derive(Clone, Copy)]
enum Side {
    Base,
    Candidate,
}

/// Per-directory matchers for both sides, loaded lazily and cached.
pub(super) struct BaseIgnore<'a> {
    repo: &'a Repository,
    base: &'a CommitSnapshot,
    workdir: PathBuf,
    /// Directory (raw bytes, `""` for the root) to its `.gitignore` entry in
    /// the base snapshot. `CommitSnapshot::read` yields nothing for a
    /// non-regular entry, so a symlinked `.gitignore` contributes no rules.
    base_files: HashMap<Vec<u8>, usize>,
    base_matchers: HashMap<Vec<u8>, Option<Gitignore>>,
    candidate_matchers: HashMap<Vec<u8>, Option<Gitignore>>,
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
            base_matchers: HashMap::new(),
            candidate_matchers: HashMap::new(),
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

    /// The verdict for `node` alone (its ancestors are not asked): the
    /// deepest `.gitignore` with a matching line decides; a whitelist line
    /// un-ignores. Every ancestor of `node` is a directory.
    fn node_ignored(
        &mut self,
        side: Side,
        node: &[u8],
        node_is_dir: bool,
    ) -> Result<bool, GitError> {
        let mut dir_end = node.iter().rposition(|byte| *byte == b'/');
        loop {
            let (dir, relative) = match dir_end {
                Some(slash) => (&node[..slash], &node[slash + 1..]),
                None => (&node[..0], node),
            };
            match self.verdict(side, dir, relative, node_is_dir)? {
                Match::Ignore(()) => return Ok(true),
                Match::Whitelist(()) => return Ok(false),
                Match::None => {}
            }
            let Some(slash) = dir_end else {
                return Ok(false);
            };
            dir_end = node[..slash].iter().rposition(|byte| *byte == b'/');
        }
    }

    /// What the `.gitignore` of `dir` says about `relative` (a path under
    /// `dir`); the matcher is built and cached on first use.
    fn verdict(
        &mut self,
        side: Side,
        dir: &[u8],
        relative: &[u8],
        node_is_dir: bool,
    ) -> Result<Match<()>, GitError> {
        let cached = match side {
            Side::Base => self.base_matchers.get(dir),
            Side::Candidate => self.candidate_matchers.get(dir),
        };
        if let Some(matcher) = cached {
            return Ok(decide(matcher.as_ref(), relative, node_is_dir));
        }
        let bytes = match side {
            Side::Base => self.read_base(dir)?,
            Side::Candidate => self.read_candidate(dir),
        };
        let built = bytes.as_deref().and_then(build_matcher);
        let decision = decide(built.as_ref(), relative, node_is_dir);
        match side {
            Side::Base => self.base_matchers.insert(dir.to_vec(), built),
            Side::Candidate => self.candidate_matchers.insert(dir.to_vec(), built),
        };
        Ok(decision)
    }

    /// The base `.gitignore` blob of `dir`; `None` when absent or over the
    /// ceiling (no rules: fails toward checking more).
    fn read_base(&self, dir: &[u8]) -> Result<Option<Vec<u8>>, GitError> {
        let Some(index) = self.base_files.get(dir) else {
            return Ok(None);
        };
        self.base.read(self.repo, &self.base.entries[*index])
    }

    /// The on-disk `.gitignore` of `dir`: regular files only (never a
    /// symlink), bounded by the ceiling; any failure means no rules.
    fn read_candidate(&self, dir: &[u8]) -> Option<Vec<u8>> {
        let mut relative = dir.to_vec();
        if !relative.is_empty() {
            relative.push(b'/');
        }
        relative.extend_from_slice(GITIGNORE_NAME);
        let full_path = repo_path_to_fs(&self.workdir, &relative);
        match fs::symlink_metadata(&full_path) {
            Ok(metadata) if metadata.is_file() => {}
            _ => return None,
        }
        let file = fs::File::open(&full_path).ok()?;
        let mut buf = Vec::new();
        file.take(SOURCE_CEILING_BYTES + 1)
            .read_to_end(&mut buf)
            .ok()?;
        (buf.len() as u64 <= SOURCE_CEILING_BYTES).then_some(buf)
    }
}

/// The match of `relative` against an optional matcher, as an owned verdict.
fn decide(matcher: Option<&Gitignore>, relative: &[u8], node_is_dir: bool) -> Match<()> {
    let Some(matcher) = matcher else {
        return Match::None;
    };
    let relative = repo_path_to_fs(Path::new(""), relative);
    match matcher.matched(&relative, node_is_dir) {
        Match::Ignore(_) => Match::Ignore(()),
        Match::Whitelist(_) => Match::Whitelist(()),
        Match::None => Match::None,
    }
}

/// A matcher from `.gitignore` bytes, decoded lossily, with one leading
/// UTF-8 BOM stripped as git does; a line the builder rejects is skipped,
/// as git skips a bad pattern.
fn build_matcher(bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(MATCHER_ROOT);
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix(UTF8_BOM).unwrap_or(&text);
    for line in text.lines() {
        let _ = builder.add_line(None, line);
    }
    builder.build().ok()
}

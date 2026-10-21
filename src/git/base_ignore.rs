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
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
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

/// Bytes that can never be a single item of a simple bracket class.
const CLASS_RESERVED: &[u8] = b"[]\\/-^";

/// A matcher from `.gitignore` bytes. One leading UTF-8 BOM is stripped as
/// git does, and each line loses a trailing CR and everything from its first
/// NUL (git reads a line as a C string). Trailing unescaped spaces are
/// trimmed as git trims them (U+0020 only).
///
/// Only a safe subset of git's pattern language reaches the `ignore`
/// matcher, because its dialect differs from git's (braces, bracket classes).
/// The subset is: literals, `*`, `?`, `**` as a whole segment, simple ASCII
/// bracket classes, `\` escapes and `/`. A line outside it, or ending in
/// non-space whitespace, never skips anything: an ignore line is dropped, and
/// a negation is widened so it re-includes at least what git re-includes
/// (trailing whitespace becomes `?`; any other cause turns each offending
/// segment into `*`, or the whole body when a construct spans a `/`). A line
/// the builder rejects is skipped, as git skips a bad pattern.
fn build_matcher(bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(MATCHER_ROOT);
    let bytes = bytes.strip_prefix(UTF8_BOM.as_slice()).unwrap_or(bytes);
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(pattern) = representable(line) {
            let _ = builder.add_line(None, &pattern);
        }
    }
    builder.build().ok()
}

/// The text to hand to the builder for one `.gitignore` line, or `None` when
/// the line must contribute no rule. The line is cut at its first NUL; a
/// comment contributes nothing; a valid UTF-8 line inside the safe subset and
/// not ending in non-space whitespace passes unchanged; anything else is
/// dropped, or widened when it is a negation.
fn representable(line: &[u8]) -> Option<String> {
    let line = line.split(|byte| *byte == 0).next().unwrap_or(line);
    if line.first() == Some(&b'#') {
        return None;
    }
    let negation = line.first() == Some(&b'!');
    let trimmed = trim_unescaped_trailing_spaces(line);
    let body = &trimmed[usize::from(negation)..];
    if let Ok(text) = std::str::from_utf8(trimmed) {
        if !ends_in_other_whitespace(text) && in_subset(body) {
            return Some(text.to_string());
        }
    }
    negation.then(|| widen_negation(body))
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

/// Whitespace that git keeps but the `ignore` matcher would trim.
fn is_other_whitespace(ch: char) -> bool {
    ch.is_whitespace() && ch != ' '
}

fn ends_in_other_whitespace(text: &str) -> bool {
    text.ends_with(is_other_whitespace)
}

/// The widened negation (with its `!`) for a body outside the safe subset.
/// Trailing non-space whitespace alone becomes `?` per UTF-8 byte (globset's
/// `?` is byte-wide), dropping the `\` that escaped the first such char.
/// Any other cause widens each offending segment to `*`.
fn widen_negation(body: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(body) {
        let tail = text.trim_end_matches(is_other_whitespace).len();
        if tail < text.len() {
            let mut prefix = &text[..tail];
            let run = prefix
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count();
            if run % 2 == 1 {
                prefix = &prefix[..prefix.len() - 1];
            }
            if in_subset(prefix.as_bytes()) {
                let mut widened = format!("!{prefix}");
                for ch in text[tail..].chars() {
                    widened.extend(std::iter::repeat_n('?', ch.len_utf8()));
                }
                return widened;
            }
        }
    }
    let widened = widen_segments(body).unwrap_or_else(|| "*".to_string());
    format!("!{widened}")
}

/// `body` with every segment outside the safe subset replaced by `*`, or
/// `None` when a construct spans a `/`.
fn widen_segments(body: &[u8]) -> Option<String> {
    let segments = split_segments(body)?;
    let last = segments.len() - 1;
    let mut widened = Vec::new();
    for (index, segment) in segments.into_iter().enumerate() {
        if index > 0 {
            widened.push(b'/');
        }
        let inside = std::str::from_utf8(segment).is_ok_and(|text| {
            segment_in_subset(text) && !(index == last && ends_in_other_whitespace(text))
        });
        if inside {
            widened.extend_from_slice(segment);
        } else {
            widened.push(b'*');
        }
    }
    String::from_utf8(widened).ok()
}

/// Whether the whole body (after any `!`) lies inside the safe subset.
fn in_subset(body: &[u8]) -> bool {
    split_segments(body).is_some_and(|segments| {
        segments
            .into_iter()
            .all(|segment| std::str::from_utf8(segment).is_ok_and(segment_in_subset))
    })
}

/// The `/`-separated segments of `body`, or `None` when an escaped `/` or a
/// `[` that opens no simple class (with a `/` after it) spans segments.
fn split_segments(body: &[u8]) -> Option<Vec<&[u8]>> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < body.len() {
        match body[index] {
            b'\\' => {
                if body.get(index + 1) == Some(&b'/') {
                    return None;
                }
                index += 2;
            }
            b'/' => {
                segments.push(&body[start..index]);
                index += 1;
                start = index;
            }
            b'[' => match class_end(body, index) {
                Some(end) => index = end + 1,
                None if body[index..].contains(&b'/') => return None,
                None => index += 1,
            },
            _ => index += 1,
        }
    }
    segments.push(&body[start..]);
    Some(segments)
}

/// Whether one segment uses only the safe subset.
fn segment_in_subset(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'}' | b']' => return false,
            b'[' => match class_end(bytes, index) {
                Some(end) => index = end + 1,
                None => return false,
            },
            b'\\' => match segment[index + 1..].chars().next() {
                Some(escaped) => index += 1 + escaped.len_utf8(),
                None => return false,
            },
            b'*' => {
                let run = bytes[index..]
                    .iter()
                    .take_while(|byte| **byte == b'*')
                    .count();
                if run > 2 || (run == 2 && segment != "**") {
                    return false;
                }
                index += run;
            }
            _ => index += 1,
        }
    }
    true
}

/// The index of the `]` closing the simple ASCII class opened at `open`: an
/// optional `!`, then single chars or `a-b` ranges of non-reserved ASCII.
fn class_end(bytes: &[u8], open: usize) -> Option<usize> {
    let item = |index: usize| {
        bytes
            .get(index)
            .copied()
            .filter(|byte| byte.is_ascii() && !CLASS_RESERVED.contains(byte))
    };
    let mut index = open + 1;
    if bytes.get(index) == Some(&b'!') {
        index += 1;
    }
    let mut items = 0;
    loop {
        if bytes.get(index) == Some(&b']') {
            return (items > 0).then_some(index);
        }
        let low = item(index)?;
        if bytes.get(index + 1) == Some(&b'-') {
            if low > item(index + 2)? {
                return None;
            }
            index += 3;
        } else {
            index += 1;
        }
        items += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::representable;

    fn text(line: &[u8]) -> Option<String> {
        representable(line)
    }

    #[test]
    fn test_representable_keeps_the_safe_subset_and_widens_the_rest() {
        let kept: [&[u8]; 11] = [
            b"*.java",
            b"out/*",
            b"!out/keep.ts",
            b"/a/**/b",
            b"**/x",
            b"x/**",
            b"Ok[a-c]?.java",
            b"[!x]y",
            b"\\#h",
            b"\\!b",
            b"a\\ ",
        ];
        for line in kept {
            let expected = String::from_utf8(line.to_vec()).expect("utf-8");
            assert_eq!(text(line), Some(expected), "{line:?}");
        }
        let dropped: [&[u8]; 6] = [
            b"Foo{a,b}",
            b"a[^b]",
            b"a[[:upper:]]",
            b"a[",
            b"a**b",
            b"a\\",
        ];
        for line in dropped {
            assert_eq!(text(line), None, "{line:?}");
        }
        let widened: [(&[u8], &str); 7] = [
            (b"!build/Gen{a,b}", "!build/*"),
            (b"!/x/a{b}/", "!/x/*/"),
            (b"!bad[a-\xff]", "!*"),
            (b"!a[x/y]b", "!*"),
            (b"!gen\\\t", "!gen?"),
            (b"!gen\\\\\t", "!gen\\\\?"),
            (b"!X.java\0junk", "!X.java"),
        ];
        for (line, expected) in widened {
            assert_eq!(text(line).as_deref(), Some(expected), "{line:?}");
        }
    }
}

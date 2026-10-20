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
    /// ignored (a child cannot be re-included) or the path itself is.
    fn ignores(&mut self, side: Side, path: &[u8], is_dir: bool) -> Result<bool, GitError> {
        let components: Vec<&[u8]> = path.split(|byte| *byte == b'/').collect();
        for node in 0..components.len() {
            let node_is_dir = node + 1 < components.len() || is_dir;
            if self.node_ignored(side, &components, node, node_is_dir)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The verdict for `components[..=node]`: the deepest `.gitignore` with
    /// a matching line decides; a whitelist line un-ignores.
    fn node_ignored(
        &mut self,
        side: Side,
        components: &[&[u8]],
        node: usize,
        node_is_dir: bool,
    ) -> Result<bool, GitError> {
        for depth in (0..=node).rev() {
            let dir = components[..depth].join(&b'/');
            let relative = components[depth..=node].join(&b'/');
            let Some(matcher) = self.matcher(side, dir)? else {
                continue;
            };
            let relative = repo_path_to_fs(Path::new(""), &relative);
            match matcher.matched(&relative, node_is_dir) {
                Match::Ignore(_) => return Ok(true),
                Match::Whitelist(_) => return Ok(false),
                Match::None => {}
            }
        }
        Ok(false)
    }

    fn matcher(&mut self, side: Side, dir: Vec<u8>) -> Result<Option<&Gitignore>, GitError> {
        let cached = match side {
            Side::Base => self.base_matchers.contains_key(&dir),
            Side::Candidate => self.candidate_matchers.contains_key(&dir),
        };
        if !cached {
            let bytes = match side {
                Side::Base => self.read_base(&dir)?,
                Side::Candidate => self.read_candidate(&dir),
            };
            let built = bytes.as_deref().and_then(build_matcher);
            match side {
                Side::Base => self.base_matchers.insert(dir.clone(), built),
                Side::Candidate => self.candidate_matchers.insert(dir.clone(), built),
            };
        }
        let cache = match side {
            Side::Base => &self.base_matchers,
            Side::Candidate => &self.candidate_matchers,
        };
        Ok(cache.get(&dir).and_then(Option::as_ref))
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

/// A matcher from `.gitignore` bytes, decoded lossily; a line the builder
/// rejects is skipped, as git skips a bad pattern.
fn build_matcher(bytes: &[u8]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(MATCHER_ROOT);
    for line in String::from_utf8_lossy(bytes).lines() {
        let _ = builder.add_line(None, line);
    }
    builder.build().ok()
}

//! Central diff / line-mapping primitive (D8-D11, D24): one file-level
//! change list between two WS-1 snapshots (added, deleted, modified,
//! renamed, type-changed), plus one line-mapping function between a base
//! and a candidate byte buffer. "Centralize line mapping as one shared
//! primitive for parse errors, findings, suppressions, callables and clone
//! attribution" (`nsd-plan-final.md` *M1-M2*).

use std::collections::{BTreeMap, BTreeSet};

use git2::{
    Delta, Diff, DiffDelta, DiffFile, DiffFindOptions, DiffLineType, DiffOptions, FileMode, Index,
    IndexEntry, IndexTime, Oid, Patch, Repository, Tree,
};

use super::path::RepoPath;
use super::snapshot::{EntryKind, WorktreeSnapshot};
use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

const MODE_REGULAR: u32 = 0o100644;
const MODE_EXECUTABLE: u32 = 0o100755;
const MODE_SYMLINK: u32 = 0o120000;
const MODE_SUBMODULE: u32 = 0o160000;

/// D8: the one value passed to libgit2's find-similar options. Renames are
/// detected for regular files only (A6); `nsd-plan-final.md` *Amendments*
/// A6 and `nsd-plan-implementation.md` *Policy and analysis behavior*.
pub const RENAME_THRESHOLD: u16 = 50;

/// D24: highest priority so every `Repository::blob` write on the reopened
/// handle lands in memory only (git2-0.21.0 `odb.rs:272`
/// `add_new_mempack_backend` and its test `write_with_mempack`, `odb.rs:710`,
/// use the same value).
const MEMPACK_BACKEND_PRIORITY: i32 = 1000;

/// A context radius large enough that libxdiff always merges every hunk in
/// a realistic source file into one, so `map_lines` returns a complete
/// base-to-candidate mapping rather than only the lines within a few rows
/// of a change.
const FULL_FILE_CONTEXT_LINES: u32 = u32::MAX;

/// One file-level change between a base tree and a candidate snapshot (D10:
/// sorted by raw path bytes). A symlink or gitlink change is reported by
/// its `EntryKind`, never line-diffed as source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added {
        path: RepoPath,
        kind: EntryKind,
    },
    Deleted {
        path: RepoPath,
        kind: EntryKind,
    },
    Modified {
        path: RepoPath,
        kind: EntryKind,
    },
    Renamed {
        from: RepoPath,
        to: RepoPath,
        kind: EntryKind,
    },
    Typechange {
        path: RepoPath,
        old_kind: EntryKind,
        new_kind: EntryKind,
    },
}

impl Change {
    /// D10: the raw-byte path a change sorts by — the post-change path for
    /// a rename, since that is the path that continues to exist.
    fn sort_key(&self) -> &RepoPath {
        match self {
            Change::Added { path, .. }
            | Change::Deleted { path, .. }
            | Change::Modified { path, .. }
            | Change::Typechange { path, .. } => path,
            Change::Renamed { to, .. } => to,
        }
    }
}

/// Diffs a base commit (`None` for an unborn `HEAD`'s empty tree, D2) to
/// another commit (`--base`).
pub fn diff_commit_to_commit(
    repo: &Repository,
    base: Option<Oid>,
    candidate: Oid,
) -> Result<Vec<Change>, GitError> {
    let base_tree = resolve_tree(repo, base)?;
    let candidate_commit = repo
        .find_commit(candidate)
        .map_err(|err| wrap_git_error("cannot read candidate commit", &err))?;
    let candidate_tree = candidate_commit
        .tree()
        .map_err(|err| wrap_git_error("cannot read candidate commit tree", &err))?;

    let mut opts = diff_options();
    let mut diff = repo
        .diff_tree_to_tree(base_tree.as_ref(), Some(&candidate_tree), Some(&mut opts))
        .map_err(|err| wrap_git_error("cannot diff commit to commit", &err))?;
    changes_from_diff(&mut diff)
}

/// Diffs a base commit (`None` for an unborn `HEAD`'s empty tree, D2) to the
/// repository's on-disk Git index (`--staged`).
pub fn diff_commit_to_index(repo: &Repository, base: Option<Oid>) -> Result<Vec<Change>, GitError> {
    let base_tree = resolve_tree(repo, base)?;
    let index = repo
        .index()
        .map_err(|err| wrap_git_error("cannot read the Git index", &err))?;

    let mut opts = diff_options();
    let mut diff = repo
        .diff_tree_to_index(base_tree.as_ref(), Some(&index), Some(&mut opts))
        .map_err(|err| wrap_git_error("cannot diff commit to index", &err))?;
    changes_from_diff(&mut diff)
}

/// Diffs a base commit (`None` for an unborn `HEAD`'s empty tree, D2) to the
/// WS-1 Worktree snapshot (`--base --worktree`).
///
/// D24: never a libgit2 workdir diff (`diff_tree_to_workdir_with_index`
/// applies clean/CRLF filters, contradicting D5, and knows nothing of the
/// WS-1 nested-checkout entry, D7). Instead this opens a second
/// `Repository` handle on the same repository, attaches an in-memory
/// mempack object backend to it, writes each regular/executable entry's
/// source bytes and each symlink's target bytes there, builds an in-memory
/// `Index` holding one entry per snapshot entry, and diffs the base tree
/// against that index. Nothing reaches the repository's on-disk ODB, index
/// or refs (D2); the mempack dies with the second handle when this
/// function returns.
pub fn diff_commit_to_worktree(
    repo: &Repository,
    base: Option<Oid>,
    worktree: &WorktreeSnapshot,
) -> Result<Vec<Change>, GitError> {
    let candidate_repo = Repository::open(repo.path()).map_err(|err| {
        wrap_git_error("cannot reopen the repository for an in-memory diff", &err)
    })?;
    candidate_repo
        .odb()
        .map_err(|err| wrap_git_error("cannot open the object database", &err))?
        .add_new_mempack_backend(MEMPACK_BACKEND_PRIORITY)
        .map_err(|err| wrap_git_error("cannot attach an in-memory object backend", &err))?;

    let mut index =
        Index::new().map_err(|err| wrap_git_error("cannot create an in-memory index", &err))?;
    let mut nested_checkouts = Vec::new();

    for entry in &worktree.entries {
        match entry.kind {
            EntryKind::NestedCheckout => {
                // D24/D7: surfaced by kind, never added to the libgit2 diff.
                nested_checkouts.push(Change::Added {
                    path: entry.path.clone(),
                    kind: EntryKind::NestedCheckout,
                });
            }
            EntryKind::Submodule => {
                let oid = entry.oid.ok_or_else(|| {
                    GitError::new(
                        CODE_SNAPSHOT_UNAVAILABLE,
                        format!(
                            "worktree submodule entry {} has no gitlink target",
                            entry.path.render()
                        ),
                    )
                })?;
                insert_index_entry(&mut index, &entry.path, MODE_SUBMODULE, entry.size, oid)?;
            }
            EntryKind::Symlink => {
                // Over SOURCE_CEILING_BYTES: an empty blob, so the diff still
                // reports a change (never a silent pass-through) without ever
                // reading the target (perf/security HIGH).
                let target = worktree.link_target(repo, entry)?.unwrap_or_default();
                let oid = candidate_repo.blob(&target).map_err(|err| {
                    wrap_git_error(
                        "cannot write a symlink target to the in-memory object store",
                        &err,
                    )
                })?;
                insert_index_entry(&mut index, &entry.path, MODE_SYMLINK, entry.size, oid)?;
            }
            EntryKind::Regular | EntryKind::Executable => {
                // Same fallback and reason as the symlink arm above.
                let content = worktree.read(repo, entry)?.unwrap_or_default();
                let oid = candidate_repo.blob(&content).map_err(|err| {
                    wrap_git_error(
                        "cannot write worktree file content to the in-memory object store",
                        &err,
                    )
                })?;
                let mode = if entry.kind == EntryKind::Executable {
                    MODE_EXECUTABLE
                } else {
                    MODE_REGULAR
                };
                insert_index_entry(&mut index, &entry.path, mode, entry.size, oid)?;
            }
        }
    }

    let base_tree = resolve_tree(&candidate_repo, base)?;
    let mut opts = diff_options();
    let mut diff = candidate_repo
        .diff_tree_to_index(base_tree.as_ref(), Some(&index), Some(&mut opts))
        .map_err(|err| wrap_git_error("cannot diff commit to worktree", &err))?;

    let mut changes = changes_from_diff(&mut diff)?;
    changes.append(&mut nested_checkouts);
    changes.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    Ok(changes)
}

/// A complete mapping between a base and a candidate byte buffer (D11:
/// 1-based `usize` line numbers, matching `src/model.rs:174` and
/// `src/exec_lines.rs:42`): base line -> candidate line for every unchanged
/// line, plus the sets of added candidate lines and deleted base lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineMap {
    pub base_to_candidate: BTreeMap<usize, usize>,
    pub added_candidate_lines: BTreeSet<usize>,
    pub deleted_base_lines: BTreeSet<usize>,
}

/// Maps lines between `base` and `candidate` (D9: forces text mode so
/// NUL-containing sources still yield hunks; encoding validity is a
/// separate A102 concern, M3).
pub fn map_lines(base: &[u8], candidate: &[u8]) -> Result<LineMap, GitError> {
    if base == candidate {
        return Ok(identity_line_map(base));
    }

    let mut opts = DiffOptions::new();
    opts.force_text(true).context_lines(FULL_FILE_CONTEXT_LINES);
    let patch = Patch::from_buffers(base, None, candidate, None, Some(&mut opts))
        .map_err(|err| wrap_git_error("cannot compute a line map between two buffers", &err))?;

    let mut map = LineMap::default();
    for hunk_idx in 0..patch.num_hunks() {
        let line_count = patch
            .num_lines_in_hunk(hunk_idx)
            .map_err(|err| wrap_git_error("cannot read a diff hunk's line count", &err))?;
        for line_idx in 0..line_count {
            let line = patch
                .line_in_hunk(hunk_idx, line_idx)
                .map_err(|err| wrap_git_error("cannot read a diff hunk's line", &err))?;
            match line.origin_value() {
                DiffLineType::Context => {
                    if let (Some(old), Some(new)) = (line.old_lineno(), line.new_lineno()) {
                        map.base_to_candidate.insert(old as usize, new as usize);
                    }
                }
                DiffLineType::Addition => {
                    if let Some(new) = line.new_lineno() {
                        map.added_candidate_lines.insert(new as usize);
                    }
                }
                DiffLineType::Deletion => {
                    if let Some(old) = line.old_lineno() {
                        map.deleted_base_lines.insert(old as usize);
                    }
                }
                // EOFNL/header/binary markers carry no line of their own.
                _ => {}
            }
        }
    }
    Ok(map)
}

/// The identity mapping for byte-identical buffers: every line maps to
/// itself, nothing is added or deleted. Short-circuits `map_lines` around
/// libgit2, which reports zero hunks for identical buffers rather than one
/// all-context hunk.
fn identity_line_map(buffer: &[u8]) -> LineMap {
    let count = count_lines(buffer);
    let mut base_to_candidate = BTreeMap::new();
    for line in 1..=count {
        base_to_candidate.insert(line, line);
    }
    LineMap {
        base_to_candidate,
        added_candidate_lines: BTreeSet::new(),
        deleted_base_lines: BTreeSet::new(),
    }
}

/// D11: a buffer's 1-based line count. A final line without a trailing
/// `\n` is still counted (`line_map_no_trailing_newline`).
fn count_lines(buffer: &[u8]) -> usize {
    if buffer.is_empty() {
        return 0;
    }
    let newline_count = buffer.iter().filter(|&&byte| byte == b'\n').count();
    if buffer.last() == Some(&b'\n') {
        newline_count
    } else {
        newline_count + 1
    }
}

/// Shared `DiffOptions` for every file-level Change listing: a type change
/// (e.g. symlink -> regular file at the same path) is one `Typechange`
/// delta, never a delete+add pair that would feed unrelated content into
/// rename-similarity scoring.
fn diff_options() -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    opts
}

/// The tree of commit `base`, or "no tree" (D2) for an unborn `HEAD`.
fn resolve_tree<'repo>(
    repo: &'repo Repository,
    base: Option<Oid>,
) -> Result<Option<Tree<'repo>>, GitError> {
    let Some(oid) = base else {
        return Ok(None);
    };
    let commit = repo
        .find_commit(oid)
        .map_err(|err| wrap_git_error("cannot read base commit", &err))?;
    let tree = commit
        .tree()
        .map_err(|err| wrap_git_error("cannot read base commit tree", &err))?;
    Ok(Some(tree))
}

/// Runs rename detection at `RENAME_THRESHOLD` (D8) and converts every
/// resulting delta to a `Change`, sorted by raw path bytes (D10).
fn changes_from_diff(diff: &mut Diff<'_>) -> Result<Vec<Change>, GitError> {
    let mut find_opts = DiffFindOptions::new();
    find_opts.renames(true).rename_threshold(RENAME_THRESHOLD);
    diff.find_similar(Some(&mut find_opts))
        .map_err(|err| wrap_git_error("cannot detect renamed files", &err))?;

    let mut changes = Vec::new();
    for delta in diff.deltas() {
        changes.push(change_from_delta(&delta)?);
    }
    changes.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    Ok(changes)
}

fn change_from_delta(delta: &DiffDelta<'_>) -> Result<Change, GitError> {
    match delta.status() {
        Delta::Added => Ok(Change::Added {
            path: repo_path_from_file(&delta.new_file())?,
            kind: entry_kind_from_file_mode(delta.new_file().mode())?,
        }),
        Delta::Deleted => Ok(Change::Deleted {
            path: repo_path_from_file(&delta.old_file())?,
            kind: entry_kind_from_file_mode(delta.old_file().mode())?,
        }),
        Delta::Modified => Ok(Change::Modified {
            path: repo_path_from_file(&delta.new_file())?,
            kind: entry_kind_from_file_mode(delta.new_file().mode())?,
        }),
        Delta::Renamed => Ok(Change::Renamed {
            from: repo_path_from_file(&delta.old_file())?,
            to: repo_path_from_file(&delta.new_file())?,
            kind: entry_kind_from_file_mode(delta.new_file().mode())?,
        }),
        Delta::Typechange => Ok(Change::Typechange {
            path: repo_path_from_file(&delta.new_file())?,
            old_kind: entry_kind_from_file_mode(delta.old_file().mode())?,
            new_kind: entry_kind_from_file_mode(delta.new_file().mode())?,
        }),
        other => Err(GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!("diff produced an unexpected delta status: {other:?}"),
        )),
    }
}

fn repo_path_from_file(file: &DiffFile<'_>) -> Result<RepoPath, GitError> {
    file.path_bytes()
        .map(|bytes| RepoPath::from_bytes(bytes.to_vec()))
        .ok_or_else(|| GitError::new(CODE_SNAPSHOT_UNAVAILABLE, "a diff delta file has no path"))
}

fn entry_kind_from_file_mode(mode: FileMode) -> Result<EntryKind, GitError> {
    match mode {
        FileMode::Blob | FileMode::BlobGroupWritable => Ok(EntryKind::Regular),
        FileMode::BlobExecutable => Ok(EntryKind::Executable),
        FileMode::Link => Ok(EntryKind::Symlink),
        FileMode::Commit => Ok(EntryKind::Submodule),
        FileMode::Tree | FileMode::Unreadable => Err(GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!("diff delta file has an unexpected mode: {mode:?}"),
        )),
    }
}

fn insert_index_entry(
    index: &mut Index,
    path: &RepoPath,
    mode: u32,
    size: u64,
    oid: Oid,
) -> Result<(), GitError> {
    let file_size = if size > u64::from(u32::MAX) {
        u32::MAX
    } else {
        size as u32
    };
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size,
        id: oid,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index
        .add(&entry)
        .map_err(|err| wrap_git_error("cannot add an in-memory index entry", &err))
}

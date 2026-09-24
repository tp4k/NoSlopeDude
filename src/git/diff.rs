//! Central diff / line-mapping primitive (D8-D11, D24): one file-level
//! change list between two WS-1 snapshots (added, deleted, modified,
//! renamed, type-changed), plus one line-mapping function between a base
//! and a candidate byte buffer. "Centralize line mapping as one shared
//! primitive for parse errors, findings, suppressions, callables and clone
//! attribution" (`nsd-plan-final.md` *M1-M2*).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use git2::{
    Delta, Diff, DiffDelta, DiffFile, DiffFindOptions, DiffLineType, DiffOptions, ErrorCode, Index,
    IndexEntry, IndexTime, ObjectType, Oid, Patch, Repository, Tree,
};

use super::path::RepoPath;
use super::snapshot::{repo_path_to_fs, Entry, EntryKind, WorktreeSnapshot, SOURCE_CEILING_BYTES};
use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

const MODE_REGULAR: u32 = 0o100644;
const MODE_EXECUTABLE: u32 = 0o100755;
const MODE_SYMLINK: u32 = 0o120000;
const MODE_SUBMODULE: u32 = 0o160000;

/// D8: the one value passed to libgit2's find-similar options. Renames are
/// detected for regular files only (A6); `nsd-plan-final.md` *Amendments*
/// A6 and `nsd-plan-implementation.md` *Policy and analysis behavior*.
pub const RENAME_THRESHOLD: u16 = 50;

/// libgit2 scores an exact-OID match at 100 without reading any content
/// (`similarity_measure`, vendored `diff_tform.c`).
const EXACT_RENAME_SIMILARITY: u16 = 100;

/// libgit2's own built-in rename-candidate cap (`DEFAULT_RENAME_LIMIT`,
/// vendored `diff_tform.c:244`), set explicitly so a user's
/// `diff.renamelimit` config can never change nsd's rename output (D8/D10:
/// deterministic renames, independent of the caller's Git config). Left
/// unset, `git_diff_find_similar` reads that config value itself whenever
/// `rename_limit` is `0` (`diff_tform.c:322-330`).
const DEFAULT_RENAME_LIMIT: usize = 1000;

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
        similarity: u16,
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
    changes_from_diff(
        &mut diff,
        base_tree.as_ref(),
        NewFileSource::Tree(&candidate_tree),
    )
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
    changes_from_diff(&mut diff, base_tree.as_ref(), NewFileSource::Index(&index))
}

/// Diffs a base commit (`None` for an unborn `HEAD`'s empty tree, D2) to the
/// WS-1 Worktree snapshot (`--base --worktree`).
///
/// D24: never a libgit2 workdir diff (`diff_tree_to_workdir_with_index`
/// applies clean/CRLF filters, contradicting D5, and knows nothing of the
/// WS-1 nested-checkout entry, D7). Instead this opens a second
/// `Repository` handle on the same repository and attaches an in-memory
/// mempack object backend to it. Nothing reaches the repository's on-disk
/// ODB, index or refs (D2); the mempack dies with the second handle when
/// this function returns. A regular/executable entry over
/// `SOURCE_CEILING_BYTES` is never read into memory or written to the
/// mempack; it is represented by a raw fd hash of its real on-disk bytes
/// instead (D2/D5), so it still diffs correctly against its real content.
///
/// Two phases keep the ODB write (and its `git_odb__freshen` readdir/utime
/// cost, perf HIGH) proportional to what rename detection actually needs,
/// not to the worktree's total size:
/// - **Phase 1** computes every entry's real object id with no ODB write at
///   all (`Oid::hash_object`/`hash_file`, or a submodule's own gitlink oid),
///   builds an in-memory `Index` from those ids, and diffs the base tree
///   against it. A real, matching id already makes an unchanged path
///   produce no delta, so Added/Deleted/Modified/Typechange are all correct
///   from this phase alone — only rename *detection* needs blob content.
/// - **Gate**: rename detection can only ever match an Added/Typechange
///   blob against a Deleted/Typechange delta's *old*-side blob. With no
///   such deleted blob anywhere in the diff, `find_similar` cannot produce
///   a rename no matter what the new side holds, so Phase 2 is skipped
///   entirely and the ODB is never touched.
/// - **Phase 2** (only when the gate fires) re-reads just the Added /
///   blob-Typechange worktree entries (`binary_search_by` on
///   `worktree.entries`, sorted by path) and writes their content to the
///   mempack, skipping any over-ceiling entry exactly as Phase 1 did. The
///   corresponding old-side blobs need no write: they are already in the
///   real repository's own ODB, which the mempack-backed handle still
///   reads through.
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

    let workdir = repo.workdir().ok_or_else(|| {
        GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            "repository has no worktree (bare repository)",
        )
    })?;

    let base_tree = resolve_tree(&candidate_repo, base)?;

    // Phase 1: every entry's real object id, with no ODB write anywhere.
    let mut index =
        Index::new().map_err(|err| wrap_git_error("cannot create an in-memory index", &err))?;
    let mut nested_checkouts = Vec::new();
    // D27: a `Special` path is never given its own content in the in-memory
    // index; masked to the base tree's entry (if any) instead, so libgit2's
    // diff sees no delta there, and reported directly by kind, same as
    // `NestedCheckout` above.
    let mut special_changes = Vec::new();

    for entry in &worktree.entries {
        match entry.kind {
            EntryKind::NestedCheckout => {
                // D24/D7: surfaced by kind, never added to the libgit2 diff.
                nested_checkouts.push(Change::Added {
                    path: entry.path.clone(),
                    kind: EntryKind::NestedCheckout,
                });
            }
            EntryKind::Special => {
                // D27: never read. A base-tree entry at this path is copied
                // in by its own mode/oid so it produces no delta (neither a
                // rename source nor a rename target); its absence means
                // this path is new.
                match old_side_entry(base_tree.as_ref(), entry.path.as_bytes())? {
                    Some((old_kind, old_mode, old_oid)) => {
                        insert_index_entry(&mut index, &entry.path, old_mode as u32, 0, old_oid)?;
                        special_changes.push(Change::Typechange {
                            path: entry.path.clone(),
                            old_kind,
                            new_kind: EntryKind::Special,
                        });
                    }
                    None => {
                        special_changes.push(Change::Added {
                            path: entry.path.clone(),
                            kind: EntryKind::Special,
                        });
                    }
                }
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
                // `link_target` only reports `None` for a non-symlink kind
                // (`WorktreeSnapshot::link_target`), never for this arm, so
                // an over-ceiling symlink target is unreachable: a symlink's
                // target is its own on-disk read, bounded by the OS, not by
                // SOURCE_CEILING_BYTES. `similarity_measure`'s `GIT_MODE_ISBLOB`
                // filter (`diff_tform.c`) excludes a symlink from ever being a
                // rename source/target by content anyway, so hashing it here
                // (never writing it, even in Phase 2) is enough either way.
                let target = worktree.link_target(repo, entry)?.ok_or_else(|| {
                    GitError::new(
                        CODE_SNAPSHOT_UNAVAILABLE,
                        format!(
                            "worktree symlink entry {} unexpectedly has no target bytes",
                            entry.path.render()
                        ),
                    )
                })?;
                let oid = Oid::hash_object(ObjectType::Blob, &target)
                    .map_err(|err| wrap_git_error("cannot hash a symlink target", &err))?;
                insert_index_entry(&mut index, &entry.path, MODE_SYMLINK, entry.size, oid)?;
            }
            EntryKind::Regular | EntryKind::Executable => {
                let oid = worktree_blob_oid(repo, workdir, worktree, entry)?;
                let mode = if entry.kind == EntryKind::Executable {
                    MODE_EXECUTABLE
                } else {
                    MODE_REGULAR
                };
                insert_index_entry(&mut index, &entry.path, mode, entry.size, oid)?;
            }
        }
    }

    let mut opts = diff_options();
    let mut diff = candidate_repo
        .diff_tree_to_index(base_tree.as_ref(), Some(&index), Some(&mut opts))
        .map_err(|err| wrap_git_error("cannot diff commit to worktree", &err))?;

    // Gate: any Deleted/Typechange delta whose *old* side is a blob is a
    // possible rename source, so Phase 2 must run.
    let needs_phase_two = has_blob_rename_source(&diff, base_tree.as_ref())?;

    // Row 1: an Added/Typechange worktree entry whose real bytes were never
    // read (over SOURCE_CEILING_BYTES) can never be written to the mempack,
    // so asking `find_similar` to score it fails with `NotFound` and, left
    // uncaught, degrades every *other* rename in the same diff. Each such
    // path is pulled out of the diff before rename detection runs and
    // resolved afterwards: an Added entry by comparing its own already-known
    // real object id against each remaining Deleted delta's old-side blob id
    // (the only similarity an unwritten blob can ever prove); a Typechange
    // entry has no such old-side blob to compare (its old side may be a
    // symlink or gitlink), so it is matched by path instead, against the
    // same-path Deleted delta the re-diff below produces.
    let mut over_ceiling: Vec<(RepoPath, EntryKind, Oid, Delta)> = Vec::new();

    if needs_phase_two {
        // Phase 2: write only the Added / blob-Typechange worktree entries'
        // content, so `find_similar` can inflate them against the old-side
        // blobs already sitting in the real repository's own ODB.
        for delta in diff.deltas() {
            let status = delta.status();
            if !matches!(status, Delta::Added | Delta::Typechange) {
                continue;
            }
            let path = repo_path_from_file(&delta.new_file())?;
            let kind = new_side_kind(&NewFileSource::Index(&index), path.as_bytes())?;
            if !matches!(kind, EntryKind::Regular | EntryKind::Executable) {
                continue;
            }
            let Ok(entry_idx) = worktree.entries.binary_search_by(|e| e.path.cmp(&path)) else {
                continue;
            };
            let entry = &worktree.entries[entry_idx];
            // `worktree.read` itself reports `None` for an over-ceiling
            // entry (row 1's guarantee): relying on the same already-
            // inserted real, unwritten oid to reconcile it after rename
            // detection instead of writing it.
            match worktree.read(repo, entry)? {
                Some(content) => {
                    candidate_repo.blob(&content).map_err(|err| {
                        wrap_git_error(
                            "cannot write worktree file content to the in-memory object store",
                            &err,
                        )
                    })?;
                }
                None => {
                    let oid = index
                        .get_path(&bytes_to_path(path.as_bytes()), 0)
                        .ok_or_else(|| {
                            GitError::new(
                                CODE_SNAPSHOT_UNAVAILABLE,
                                format!(
                                    "an over-ceiling worktree entry {} is missing from its own in-memory index",
                                    path.render()
                                ),
                            )
                        })?
                        .id;
                    over_ceiling.push((path, kind, oid, status));
                }
            }
        }

        if !over_ceiling.is_empty() {
            // Never let `find_similar` see these paths at all: removing
            // them keeps the lookup it performs for every other candidate
            // from ever hitting the unwritten blob.
            for (path, _, _, _) in &over_ceiling {
                index
                    .remove_path(&bytes_to_path(path.as_bytes()))
                    .map_err(|err| {
                        wrap_git_error(
                            "cannot remove an over-ceiling entry before rename detection",
                            &err,
                        )
                    })?;
            }
            diff = candidate_repo
                .diff_tree_to_index(base_tree.as_ref(), Some(&index), Some(&mut opts))
                .map_err(|err| {
                    wrap_git_error(
                        "cannot re-diff commit to worktree without over-ceiling adds",
                        &err,
                    )
                })?;
        }
    }

    let mut changes =
        changes_from_diff(&mut diff, base_tree.as_ref(), NewFileSource::Index(&index))?;

    for (path, kind, oid, status) in over_ceiling {
        if status == Delta::Typechange {
            // A Typechange's old side is never a blob (row 1's comment
            // above), so it cannot be matched by object id like an Added
            // entry: match the same-path Deleted delta the re-diff above
            // produced instead, and rebuild the one Typechange it replaces.
            let idx = changes
                .iter()
                .position(
                    |change| matches!(change, Change::Deleted { path: from, .. } if *from == path),
                )
                .ok_or_else(|| {
                    GitError::new(
                        CODE_SNAPSHOT_UNAVAILABLE,
                        format!(
                            "an over-ceiling typechange entry {} has no matching deleted old side",
                            path.render()
                        ),
                    )
                })?;
            let Change::Deleted { kind: old_kind, .. } = changes.remove(idx) else {
                unreachable!("matched idx only ever indexes a Change::Deleted");
            };
            changes.push(Change::Typechange {
                path,
                old_kind,
                new_kind: kind,
            });
            continue;
        }

        let matched_idx = changes.iter().position(|change| match change {
            Change::Deleted {
                path: from,
                kind: EntryKind::Regular | EntryKind::Executable,
            } => old_side_blob_oid(base_tree.as_ref(), from.as_bytes()) == Some(oid),
            _ => false,
        });
        match matched_idx {
            Some(idx) => {
                let Change::Deleted { path: from, .. } = changes.remove(idx) else {
                    unreachable!("matched_idx only ever indexes a Change::Deleted");
                };
                changes.push(Change::Renamed {
                    from,
                    to: path,
                    kind,
                    similarity: EXACT_RENAME_SIMILARITY,
                });
            }
            None => changes.push(Change::Added { path, kind }),
        }
    }

    changes.append(&mut nested_checkouts);
    changes.append(&mut special_changes);
    changes.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    Ok(changes)
}

/// Phase 1's real object id for a Regular/Executable worktree entry, with no
/// ODB write: `Oid::hash_object` on its bytes when they read within
/// `SOURCE_CEILING_BYTES`, or, over that ceiling, row 1's raw fd hash of the
/// real on-disk bytes (no filters, D5; no ODB write, D2). `worktree.read`'s
/// `None` also covers a Regular entry swapped for a non-regular file after
/// enumeration (D25's re-check): the `fs::symlink_metadata` re-check here
/// tells the two cases apart, so a swapped-in FIFO/socket is never handed to
/// `Oid::hash_file`, which would `open(2)` it and block.
fn worktree_blob_oid(
    repo: &Repository,
    workdir: &Path,
    worktree: &WorktreeSnapshot,
    entry: &Entry,
) -> Result<Oid, GitError> {
    match worktree.read(repo, entry)? {
        Some(content) => Oid::hash_object(ObjectType::Blob, &content)
            .map_err(|err| wrap_git_error("cannot hash worktree file content", &err)),
        None => {
            let fs_path = repo_path_to_fs(workdir, entry.path.as_bytes());
            match fs::symlink_metadata(&fs_path) {
                Ok(metadata) if metadata.is_file() => {}
                _ => {
                    return Err(GitError::new(
                        CODE_SNAPSHOT_UNAVAILABLE,
                        format!(
                            "worktree entry {} is no longer a regular file",
                            entry.path.render()
                        ),
                    ))
                }
            }
            Oid::hash_file(ObjectType::Blob, &fs_path)
                .map_err(|err| wrap_git_error("cannot hash an over-ceiling worktree file", &err))
        }
    }
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
/// rename-similarity scoring. `max_size` caps both sides at
/// `SOURCE_CEILING_BYTES` so a non-exact renamed delta's `Patch::to_buf`
/// (`rename_similarity`) never inflates an over-ceiling blob to build the
/// diff text it discards.
fn diff_options() -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    opts.max_size(SOURCE_CEILING_BYTES as i64);
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

/// Whether `diff` has any Deleted/Typechange delta whose *old* side is a
/// regular/executable blob - the only shape `find_similar` can ever match a
/// rename candidate against, so Phase 2 of `diff_commit_to_worktree` only
/// needs to run when this is true.
fn has_blob_rename_source(diff: &Diff<'_>, base_tree: Option<&Tree<'_>>) -> Result<bool, GitError> {
    for delta in diff.deltas() {
        if !matches!(delta.status(), Delta::Deleted | Delta::Typechange) {
            continue;
        }
        let path = repo_path_from_file(&delta.old_file())?;
        let kind = old_side_kind(base_tree, path.as_bytes())?;
        if matches!(kind, EntryKind::Regular | EntryKind::Executable) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The old-side blob's object id at `path_bytes` in `base_tree`, or `None`
/// when `base_tree` is absent (D2's unborn-HEAD case) or has no entry
/// there. Used by row 1's over-ceiling exact-rename reconciliation, which
/// can only ever prove a match by object id.
fn old_side_blob_oid(base_tree: Option<&Tree<'_>>, path_bytes: &[u8]) -> Option<Oid> {
    base_tree?
        .get_path(&bytes_to_path(path_bytes))
        .ok()
        .map(|entry| entry.id())
}

/// The old-side entry (kind, raw mode, object id) at `path_bytes` in
/// `base_tree`, or `None` when `base_tree` is absent (D2's unborn-HEAD case)
/// or has no entry there. D27's mask for a `Special` worktree entry: copying
/// this triple into the in-memory index keeps a base blob out of rename
/// detection instead of surfacing it as a spurious Deleted delta.
fn old_side_entry(
    base_tree: Option<&Tree<'_>>,
    path_bytes: &[u8],
) -> Result<Option<(EntryKind, i32, Oid)>, GitError> {
    let Some(tree) = base_tree else {
        return Ok(None);
    };
    match tree.get_path(&bytes_to_path(path_bytes)) {
        Ok(entry) => {
            let mode = entry.filemode();
            let kind = entry_kind_from_raw_mode(mode)?;
            Ok(Some((kind, mode, entry.id())))
        }
        Err(err) if err.code() == ErrorCode::NotFound => Ok(None),
        Err(err) => Err(wrap_git_error("cannot read a base tree entry", &err)),
    }
}

/// Where a diff's new-side file mode is read from: a candidate commit's
/// tree (`diff_commit_to_commit`), or a Git index (`diff_commit_to_index`,
/// `diff_commit_to_worktree`). The old side is always a tree (or "no tree"
/// for an unborn `HEAD`, D2), so it needs no such distinction.
enum NewFileSource<'a, 'repo> {
    Tree(&'a Tree<'repo>),
    Index(&'a Index),
}

/// Runs rename detection at `RENAME_THRESHOLD` (D8) and converts every
/// resulting delta to a `Change`, sorted by raw path bytes (D10).
fn changes_from_diff(
    diff: &mut Diff<'_>,
    base_tree: Option<&Tree<'_>>,
    new_source: NewFileSource<'_, '_>,
) -> Result<Vec<Change>, GitError> {
    let mut find_opts = DiffFindOptions::new();
    find_opts
        .renames(true)
        .rename_threshold(RENAME_THRESHOLD)
        .rename_limit(DEFAULT_RENAME_LIMIT);
    diff.find_similar(Some(&mut find_opts))
        .map_err(|err| wrap_git_error("cannot detect renamed files", &err))?;

    let mut changes = Vec::new();
    for (idx, delta) in diff.deltas().enumerate() {
        changes.push(change_from_delta(
            diff,
            idx,
            &delta,
            base_tree,
            &new_source,
        )?);
    }
    changes.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));
    Ok(changes)
}

/// `Delta::Renamed`'s similarity (D8): libgit2's own `delta->similarity`
/// via a per-delta `Patch`, never `Diff::print`/`Diff::foreach` (both build
/// a full patch, and so a full blob read, for every delta). Both blobs are
/// size-capped by `diff_options`'s `max_size` (`SOURCE_CEILING_BYTES`).
fn rename_similarity(diff: &Diff<'_>, idx: usize) -> Result<u16, GitError> {
    let mut patch = Patch::from_diff(diff, idx)
        .map_err(|err| wrap_git_error("cannot build a patch for a renamed delta", &err))?
        .ok_or_else(|| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                "a Renamed delta produced no patch",
            )
        })?;
    let text = patch
        .to_buf()
        .map_err(|err| wrap_git_error("cannot render a renamed delta's patch text", &err))?;
    // Scanned as bytes, never decoded whole (the hunk lines are raw file
    // content, so a text-classified rename can still hold invalid UTF-8):
    // only the digits after the header prefix need to be valid UTF-8.
    const SIMILARITY_PREFIX: &[u8] = b"similarity index ";
    for line in text.split(|&byte| byte == b'\n') {
        let Some(rest) = line.strip_prefix(SIMILARITY_PREFIX) else {
            continue;
        };
        let Some(digits) = rest.strip_suffix(b"%") else {
            continue;
        };
        if let Ok(digits) = std::str::from_utf8(digits) {
            if let Ok(similarity) = digits.parse::<u16>() {
                return Ok(similarity);
            }
        }
    }
    Err(GitError::new(
        CODE_SNAPSHOT_UNAVAILABLE,
        "a renamed delta's patch text has no similarity index header",
    ))
}

fn change_from_delta(
    diff: &Diff<'_>,
    idx: usize,
    delta: &DiffDelta<'_>,
    base_tree: Option<&Tree<'_>>,
    new_source: &NewFileSource<'_, '_>,
) -> Result<Change, GitError> {
    match delta.status() {
        Delta::Added => {
            let path = repo_path_from_file(&delta.new_file())?;
            let kind = new_side_kind(new_source, path.as_bytes())?;
            Ok(Change::Added { path, kind })
        }
        Delta::Deleted => {
            let path = repo_path_from_file(&delta.old_file())?;
            let kind = old_side_kind(base_tree, path.as_bytes())?;
            Ok(Change::Deleted { path, kind })
        }
        Delta::Modified => {
            let path = repo_path_from_file(&delta.new_file())?;
            let kind = new_side_kind(new_source, path.as_bytes())?;
            Ok(Change::Modified { path, kind })
        }
        Delta::Renamed => {
            let from = repo_path_from_file(&delta.old_file())?;
            let to = repo_path_from_file(&delta.new_file())?;
            let kind = new_side_kind(new_source, to.as_bytes())?;
            // An exact-OID rename scores 100 without libgit2 ever reading
            // either blob's content (`similarity_measure`): skip the full
            // per-delta patch build `rename_similarity` needs, which would
            // inflate both blobs just to print back a value already known.
            let similarity = if delta.old_file().id() == delta.new_file().id() {
                EXACT_RENAME_SIMILARITY
            } else {
                rename_similarity(diff, idx)?
            };
            Ok(Change::Renamed {
                from,
                to,
                kind,
                similarity,
            })
        }
        Delta::Typechange => {
            let path = repo_path_from_file(&delta.new_file())?;
            let old_kind = old_side_kind(base_tree, path.as_bytes())?;
            let new_kind = new_side_kind(new_source, path.as_bytes())?;
            Ok(Change::Typechange {
                path,
                old_kind,
                new_kind,
            })
        }
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

/// An old-side file's kind, read from `base_tree` with the normalized,
/// non-panicking `TreeEntry::filemode()` accessor (never git2's
/// `DiffFile::mode()`, which panics on a raw tree mode libgit2 has not
/// squashed to a recognized value; row 3).
fn old_side_kind(base_tree: Option<&Tree<'_>>, path_bytes: &[u8]) -> Result<EntryKind, GitError> {
    let tree = base_tree.ok_or_else(|| {
        GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            "diff delta references an old-side file with no base tree",
        )
    })?;
    let entry = tree
        .get_path(&bytes_to_path(path_bytes))
        .map_err(|err| wrap_git_error("cannot read a base tree entry", &err))?;
    entry_kind_from_raw_mode(entry.filemode())
}

/// A new-side file's kind, read from `new_source` with the same
/// normalized, non-panicking accessor as `old_side_kind` (`TreeEntry::filemode()`
/// for a tree, an index entry's own `.mode` for an index; row 3).
fn new_side_kind(
    new_source: &NewFileSource<'_, '_>,
    path_bytes: &[u8],
) -> Result<EntryKind, GitError> {
    match new_source {
        NewFileSource::Tree(tree) => {
            let entry = tree
                .get_path(&bytes_to_path(path_bytes))
                .map_err(|err| wrap_git_error("cannot read a candidate tree entry", &err))?;
            entry_kind_from_raw_mode(entry.filemode())
        }
        NewFileSource::Index(index) => {
            let entry = index
                .get_path(&bytes_to_path(path_bytes), 0)
                .ok_or_else(|| {
                    GitError::new(
                        CODE_SNAPSHOT_UNAVAILABLE,
                        "diff delta references a new-side file missing from the index",
                    )
                })?;
            entry_kind_from_raw_mode(entry.mode as i32)
        }
    }
}

fn entry_kind_from_raw_mode(mode: i32) -> Result<EntryKind, GitError> {
    match mode as u32 {
        MODE_REGULAR => Ok(EntryKind::Regular),
        MODE_EXECUTABLE => Ok(EntryKind::Executable),
        MODE_SYMLINK => Ok(EntryKind::Symlink),
        MODE_SUBMODULE => Ok(EntryKind::Submodule),
        other => Err(GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!("diff delta file has an unexpected mode: {other:o}"),
        )),
    }
}

/// Joins raw repository-relative path bytes into a `Path` for a git2 tree
/// or index lookup (never a filesystem access: contrast `repo_path_to_fs`
/// in `snapshot.rs`). Unix rebuilds the exact bytes via `OsStr::from_bytes`,
/// so no byte is ever substituted; only a platform without a byte-oriented
/// `OsStr` falls back to a lossy conversion.
#[cfg(unix)]
fn bytes_to_path(path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
fn bytes_to_path(path: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(path).as_ref())
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

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use tempfile::TempDir;

    fn init_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().expect("create a temp dir for the test repository");
        let repo = Repository::init(dir.path()).expect("init the test repository");
        (dir, repo)
    }

    /// Commits `entries` (raw path, raw mode, content) as a single flat
    /// tree, returning the new commit's id.
    fn commit_entries(repo: &Repository, entries: &[(&[u8], u32, &[u8])]) -> Oid {
        let mut builder = repo.treebuilder(None).expect("create a tree builder");
        for (path, mode, content) in entries {
            let blob_oid = repo.blob(content).expect("write a blob");
            builder
                .insert(bytes_to_path(path), blob_oid, *mode as i32)
                .expect("insert a tree entry");
        }
        let tree_oid = builder.write().expect("write the tree");
        let tree = repo.find_tree(tree_oid).expect("find the built tree");
        let signature = Signature::now("nsd test fixture", "fixture@example.invalid")
            .expect("build a signature");
        repo.commit(None, &signature, &signature, "fixture", &tree, &[])
            .expect("commit the fixture tree")
    }

    /// An in-memory index (D24) reflecting `entries` (raw path, raw mode,
    /// content used only to compute its size and blob id).
    fn build_index(repo: &Repository, entries: &[(&[u8], u32, &[u8])]) -> Index {
        let mut index = Index::new().expect("create an in-memory index");
        for (path, mode, content) in entries {
            let blob_oid = repo.blob(content).expect("write a blob");
            let path = RepoPath::from_bytes(path.to_vec());
            insert_index_entry(&mut index, &path, *mode, content.len() as u64, blob_oid)
                .expect("insert an in-memory index entry");
        }
        index
    }

    #[test]
    fn no_deleted_or_typechanged_blob_is_false() {
        // (a) Base {a.ts}, index {a.ts modified, b.ts added}: a Modified
        // and an Added delta, no Deleted/Typechange delta at all.
        let (_dir, repo) = init_repo();
        let base_oid = commit_entries(&repo, &[(b"a.ts", MODE_REGULAR, b"one\n")]);
        let base_tree = repo.find_commit(base_oid).unwrap().tree().unwrap();
        let index = build_index(
            &repo,
            &[
                (b"a.ts", MODE_REGULAR, b"two\n"),
                (b"b.ts", MODE_REGULAR, b"new\n"),
            ],
        );

        let diff = repo
            .diff_tree_to_index(Some(&base_tree), Some(&index), Some(&mut diff_options()))
            .expect("diff tree to index");

        assert!(!has_blob_rename_source(&diff, Some(&base_tree)).expect("gate check"));
    }

    #[test]
    fn a_deleted_regular_blob_is_true() {
        // (b) Base {a.ts, gone.ts}, index {a.ts}: gone.ts is a Deleted
        // delta whose old side is a regular blob.
        let (_dir, repo) = init_repo();
        let base_oid = commit_entries(
            &repo,
            &[
                (b"a.ts", MODE_REGULAR, b"one\n"),
                (b"gone.ts", MODE_REGULAR, b"bye\n"),
            ],
        );
        let base_tree = repo.find_commit(base_oid).unwrap().tree().unwrap();
        let index = build_index(&repo, &[(b"a.ts", MODE_REGULAR, b"one\n")]);

        let diff = repo
            .diff_tree_to_index(Some(&base_tree), Some(&index), Some(&mut diff_options()))
            .expect("diff tree to index");

        assert!(has_blob_rename_source(&diff, Some(&base_tree)).expect("gate check"));
    }

    #[test]
    fn a_deleted_symlink_is_false() {
        // (c) Base {link (MODE_SYMLINK)}, index {}: link is a Deleted
        // delta, but its old side is a symlink, not a regular/executable
        // blob (find_similar can never match against it).
        let (_dir, repo) = init_repo();
        let base_oid = commit_entries(&repo, &[(b"link", MODE_SYMLINK, b"target.ts")]);
        let base_tree = repo.find_commit(base_oid).unwrap().tree().unwrap();
        let index = Index::new().expect("create an empty in-memory index");

        let diff = repo
            .diff_tree_to_index(Some(&base_tree), Some(&index), Some(&mut diff_options()))
            .expect("diff tree to index");

        assert!(!has_blob_rename_source(&diff, Some(&base_tree)).expect("gate check"));
    }
}

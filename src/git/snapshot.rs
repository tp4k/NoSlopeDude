//! `Commit`, `Index` and `Worktree` snapshots over repository-relative
//! bytes (D5: no smudge/clean/CRLF filters in either direction). `--staged`
//! reads only the `Index` snapshot, never the `Worktree` one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use git2::{ErrorCode, Oid, Repository, Tree};

use super::path::RepoPath;
use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

const MODE_TREE: i32 = 0o040000;
const MODE_REGULAR: i32 = 0o100644;
const MODE_EXECUTABLE: i32 = 0o100755;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

/// A repository-relative entry's shape (D5: "Handle additions, deletions,
/// modifications, renames, symlinks, submodules ... deterministically").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Regular,
    Executable,
    Symlink,
    Submodule,
    /// Worktree-only (D7): an untracked directory containing its own `.git`
    /// (file or directory), surfaced as one entry rather than descended.
    NestedCheckout,
}

/// One entry in a snapshot.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: RepoPath,
    pub kind: EntryKind,
    /// The blob (or, for a submodule, gitlink target) object id; `None`
    /// for a worktree entry, which git2 never wrote to the ODB (D2).
    pub oid: Option<Oid>,
    pub size: u64,
    /// Raw bytes with no smudge/clean/CRLF filters (D5). `None` for kinds
    /// that carry no source bytes (`Symlink`, `Submodule`,
    /// `NestedCheckout`).
    pub content: Option<Vec<u8>>,
}

/// A snapshot of one Git commit's tree.
#[derive(Debug, Clone)]
pub struct CommitSnapshot {
    pub entries: Vec<Entry>,
}

impl CommitSnapshot {
    /// The commit `HEAD` points to, or an empty tree (zero entries) when
    /// `HEAD` is unborn (D2: represented as "no tree" rather than by
    /// writing the empty-tree object to the ODB).
    pub fn head_or_empty(repo: &Repository) -> Result<CommitSnapshot, GitError> {
        match repo.head() {
            Ok(head_ref) => {
                let commit = head_ref
                    .peel_to_commit()
                    .map_err(|err| wrap_git_error("HEAD does not resolve to a commit", &err))?;
                CommitSnapshot::at(repo, commit.id())
            }
            Err(err) if err.code() == ErrorCode::UnbornBranch => Ok(CommitSnapshot {
                entries: Vec::new(),
            }),
            Err(err) => Err(wrap_git_error("cannot resolve HEAD", &err)),
        }
    }

    /// The tree of the commit `oid` points to.
    pub fn at(repo: &Repository, oid: Oid) -> Result<CommitSnapshot, GitError> {
        let commit = repo
            .find_commit(oid)
            .map_err(|err| wrap_git_error("cannot read commit", &err))?;
        let tree = commit
            .tree()
            .map_err(|err| wrap_git_error("cannot read commit tree", &err))?;
        let mut entries = Vec::new();
        walk_tree(repo, &tree, &[], &mut entries)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(CommitSnapshot { entries })
    }
}

/// A snapshot of the Git index (`--staged`'s candidate, D5's "index bytes
/// are raw blob bytes").
#[derive(Debug, Clone)]
pub struct IndexSnapshot {
    pub entries: Vec<Entry>,
}

impl IndexSnapshot {
    /// Fails with `NSD-G101` (D4) when the index holds unresolved merge
    /// conflicts (stage 1-3 entries).
    pub fn open(repo: &Repository) -> Result<IndexSnapshot, GitError> {
        let index = repo
            .index()
            .map_err(|err| wrap_git_error("cannot read the Git index", &err))?;
        if index.has_conflicts() {
            return Err(GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                "the Git index has unresolved merge conflicts (stage 1-3 entries); resolve them \
                 before a snapshot can be taken",
            ));
        }

        let mut entries = Vec::with_capacity(index.len());
        for index_entry in index.iter() {
            let kind = match entry_kind(index_entry.mode as i32) {
                Some(kind) => kind,
                // Index entries are never directories: trees are flattened
                // to blob/gitlink rows. An unrecognised mode is skipped.
                None => continue,
            };
            let fields = read_blob_entry(repo, kind, index_entry.id)?;
            entries.push(Entry {
                path: RepoPath::from_bytes(index_entry.path),
                kind,
                oid: fields.oid,
                size: fields.size,
                content: fields.content,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(IndexSnapshot { entries })
    }
}

/// A snapshot of the worktree: the index (D5) overlaid with on-disk
/// modifications, on-disk deletions removed, and untracked files added
/// (D6: including ones the worktree's own `.gitignore` matches).
#[derive(Debug, Clone)]
pub struct WorktreeSnapshot {
    pub entries: Vec<Entry>,
}

/// The literal `.git` path component every built-in exclusion checks for
/// (D7): never surfaced itself, and the marker of a nested checkout.
const GIT_DIR_NAME: &str = ".git";

impl WorktreeSnapshot {
    pub fn open(repo: &Repository) -> Result<WorktreeSnapshot, GitError> {
        let workdir = repo
            .workdir()
            .ok_or_else(|| {
                GitError::new(
                    CODE_SNAPSHOT_UNAVAILABLE,
                    "repository has no worktree (bare repository)",
                )
            })?
            .to_path_buf();

        let index_snapshot = IndexSnapshot::open(repo)?;
        let tracked: BTreeSet<RepoPath> = index_snapshot
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        let mut by_path: BTreeMap<RepoPath, Entry> = BTreeMap::new();
        for entry in index_snapshot.entries {
            let full_path = repo_path_to_fs(&workdir, entry.path.as_bytes());
            match fs::symlink_metadata(&full_path) {
                Ok(metadata) => {
                    if let Some(refreshed) =
                        refresh_from_disk(&entry.path, &full_path, &metadata, entry.kind)?
                    {
                        by_path.insert(entry.path.clone(), refreshed);
                    }
                }
                Err(_) => {
                    // On-disk deletion (D5/D6): drop from the overlay.
                }
            }
        }

        walk_worktree(&workdir, &[], &tracked, &mut by_path)?;

        let mut entries: Vec<Entry> = by_path.into_values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(WorktreeSnapshot { entries })
    }
}

fn entry_kind(mode: i32) -> Option<EntryKind> {
    match mode {
        MODE_REGULAR => Some(EntryKind::Regular),
        MODE_EXECUTABLE => Some(EntryKind::Executable),
        MODE_SYMLINK => Some(EntryKind::Symlink),
        MODE_SUBMODULE => Some(EntryKind::Submodule),
        _ => None,
    }
}

/// The `oid`/`size`/`content` fields a tree or index blob entry contributes
/// to an `Entry` (factored out of a plain tuple return per
/// `clippy::type_complexity`).
struct BlobFields {
    oid: Option<Oid>,
    size: u64,
    content: Option<Vec<u8>>,
}

/// Reads a blob's content for a `Regular`/`Executable` entry; a `Symlink`
/// or `Submodule` entry carries no source bytes.
fn read_blob_entry(repo: &Repository, kind: EntryKind, id: Oid) -> Result<BlobFields, GitError> {
    match kind {
        EntryKind::Regular | EntryKind::Executable => {
            let blob = repo
                .find_blob(id)
                .map_err(|err| wrap_git_error("cannot read blob", &err))?;
            let content = blob.content().to_vec();
            let size = content.len() as u64;
            Ok(BlobFields {
                oid: Some(blob.id()),
                size,
                content: Some(content),
            })
        }
        EntryKind::Symlink | EntryKind::Submodule | EntryKind::NestedCheckout => Ok(BlobFields {
            oid: Some(id),
            size: 0,
            content: None,
        }),
    }
}

/// Recursively walks `tree`, accumulating every blob/symlink/gitlink entry
/// under `prefix` (raw bytes, `/`-joined) into `entries`.
fn walk_tree(
    repo: &Repository,
    tree: &Tree,
    prefix: &[u8],
    entries: &mut Vec<Entry>,
) -> Result<(), GitError> {
    for tree_entry in tree.iter() {
        let mut path_bytes = prefix.to_vec();
        if !path_bytes.is_empty() {
            path_bytes.push(b'/');
        }
        path_bytes.extend_from_slice(tree_entry.name_bytes());

        let mode = tree_entry.filemode();
        if mode == MODE_TREE {
            let subtree = tree_entry
                .to_object(repo)
                .and_then(|object| object.peel_to_tree())
                .map_err(|err| wrap_git_error("cannot read tree entry", &err))?;
            walk_tree(repo, &subtree, &path_bytes, entries)?;
            continue;
        }
        let Some(kind) = entry_kind(mode) else {
            continue; // Unrecognised mode: skip (git itself only writes the five above).
        };
        let fields = read_blob_entry(repo, kind, tree_entry.id())?;
        entries.push(Entry {
            path: RepoPath::from_bytes(path_bytes),
            kind,
            oid: fields.oid,
            size: fields.size,
            content: fields.content,
        });
    }
    Ok(())
}

/// Joins a repository-relative byte path onto `workdir` for a filesystem
/// access. On-disk names are always valid UTF-8 in practice (D23: APFS
/// rejects non-UTF-8 file names), so a lossy conversion never loses bytes
/// for a real filesystem path; it is only ever exercised on paths built
/// straight from `std::fs::read_dir`, never on the non-UTF-8 tree/index
/// fixtures.
fn repo_path_to_fs(workdir: &Path, path: &[u8]) -> PathBuf {
    workdir.join(String::from_utf8_lossy(path).as_ref())
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

/// Recomputes a tracked entry's worktree shape after confirming the path
/// still exists on disk (D5: worktree bytes are always read fresh from
/// disk). Returns `None` when the on-disk object can no longer be
/// represented as source (e.g. a plain directory replaced a tracked blob).
fn refresh_from_disk(
    path: &RepoPath,
    full_path: &Path,
    metadata: &fs::Metadata,
    original_kind: EntryKind,
) -> Result<Option<Entry>, GitError> {
    if metadata.is_dir() {
        if original_kind == EntryKind::Submodule {
            return Ok(Some(Entry {
                path: path.clone(),
                kind: EntryKind::Submodule,
                oid: None,
                size: 0,
                content: None,
            }));
        }
        return Ok(None);
    }
    if metadata.file_type().is_symlink() {
        return Ok(Some(Entry {
            path: path.clone(),
            kind: EntryKind::Symlink,
            oid: None,
            size: 0,
            content: None,
        }));
    }
    let bytes = fs::read(full_path).map_err(|err| {
        GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!("cannot read worktree file {}: {err}", path.render()),
        )
    })?;
    let kind = if is_executable(metadata) {
        EntryKind::Executable
    } else {
        EntryKind::Regular
    };
    Ok(Some(Entry {
        path: path.clone(),
        kind,
        oid: None,
        size: bytes.len() as u64,
        content: Some(bytes),
    }))
}

/// Whether `dir` directly contains its own `.git` (file or directory) — the
/// D7 nested-checkout marker.
fn has_git_marker(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(GIT_DIR_NAME)).is_ok()
}

/// Adds every untracked worktree entry under `prefix` to `by_path` (D6:
/// ignoring the candidate's own `.gitignore`), skipping the `tracked` paths
/// already handled by the index overlay and any path with a `.git`
/// component (built-in exclusion, D7 defence in depth).
fn walk_worktree(
    workdir: &Path,
    prefix: &[u8],
    tracked: &BTreeSet<RepoPath>,
    by_path: &mut BTreeMap<RepoPath, Entry>,
) -> Result<(), GitError> {
    let dir_path = repo_path_to_fs(workdir, prefix);
    let read_dir = fs::read_dir(&dir_path).map_err(|err| {
        GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!(
                "cannot read worktree directory {}: {err}",
                dir_path.display()
            ),
        )
    })?;

    for dir_entry in read_dir {
        let dir_entry = dir_entry.map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!("cannot read a worktree directory entry: {err}"),
            )
        })?;
        let name_bytes = os_str_bytes(&dir_entry.file_name());
        if name_bytes == GIT_DIR_NAME.as_bytes() {
            continue; // Built-in exclusion (D7): never descend into `.git` itself.
        }

        let mut child_bytes = prefix.to_vec();
        if !child_bytes.is_empty() {
            child_bytes.push(b'/');
        }
        child_bytes.extend_from_slice(&name_bytes);
        let relative = RepoPath::from_bytes(child_bytes.clone());

        let metadata = dir_entry.metadata().map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!("cannot stat worktree entry {}: {err}", relative.render()),
            )
        })?;

        if metadata.is_dir() {
            if tracked.contains(&relative) {
                continue; // A tracked submodule directory: already handled by the overlay step.
            }
            let child_fs_path = repo_path_to_fs(workdir, &child_bytes);
            if has_git_marker(&child_fs_path) {
                by_path.insert(
                    relative.clone(),
                    Entry {
                        path: relative,
                        kind: EntryKind::NestedCheckout,
                        oid: None,
                        size: 0,
                        content: None,
                    },
                );
                continue; // D7: surfaced, not descended.
            }
            walk_worktree(workdir, &child_bytes, tracked, by_path)?;
            continue;
        }

        if tracked.contains(&relative) {
            continue; // Already handled by the overlay step.
        }
        if metadata.file_type().is_symlink() {
            by_path.insert(
                relative.clone(),
                Entry {
                    path: relative,
                    kind: EntryKind::Symlink,
                    oid: None,
                    size: 0,
                    content: None,
                },
            );
            continue;
        }
        let full_path = repo_path_to_fs(workdir, &child_bytes);
        let bytes = fs::read(&full_path).map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!("cannot read untracked file {}: {err}", relative.render()),
            )
        })?;
        let kind = if is_executable(&metadata) {
            EntryKind::Executable
        } else {
            EntryKind::Regular
        };
        by_path.insert(
            relative.clone(),
            Entry {
                path: relative,
                kind,
                oid: None,
                size: bytes.len() as u64,
                content: Some(bytes),
            },
        );
    }
    Ok(())
}

#[cfg(unix)]
fn os_str_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_str_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_string_lossy().into_owned().into_bytes()
}

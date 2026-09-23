//! `Commit`, `Index` and `Worktree` snapshots over repository-relative
//! bytes (D5: no smudge/clean/CRLF filters in either direction). `--staged`
//! reads only the `Index` snapshot, never the `Worktree` one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use git2::{ErrorCode, Odb, Oid, Repository, Tree};

use super::path::RepoPath;
use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

const MODE_TREE: i32 = 0o040000;
const MODE_REGULAR: i32 = 0o100644;
const MODE_EXECUTABLE: i32 = 0o100755;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

/// The largest source blob/file this crate will read into memory (perf
/// HIGH: an attacker-controlled blob must not OOM the check). A `read`
/// past this ceiling reports `Ok(None)` instead; `size` is still exact.
pub const SOURCE_CEILING_BYTES: u64 = 1_048_576;

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

/// One entry in a snapshot. Carries no source bytes itself (perf HIGH: an
/// eagerly loaded `content` field made every snapshot proportional to the
/// repository's total blob size, and let an attacker-controlled blob OOM
/// the check); call `read`/`link_target` on the owning snapshot to fetch
/// bytes for one entry, bounded by `SOURCE_CEILING_BYTES`.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: RepoPath,
    pub kind: EntryKind,
    /// The blob (or, for a submodule, gitlink target) object id; `None`
    /// for a worktree entry, which git2 never wrote to the ODB (D2).
    pub oid: Option<Oid>,
    pub size: u64,
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
        // Opened once per snapshot (not per entry): `size` comes from the
        // ODB header, never a full blob inflate (perf HIGH).
        let odb = repo
            .odb()
            .map_err(|err| wrap_git_error("cannot open the object database", &err))?;
        let mut entries = Vec::new();
        walk_tree(repo, &odb, &tree, &[], &mut entries)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(CommitSnapshot { entries })
    }

    /// Source bytes for a `Regular`/`Executable` entry, bounded by
    /// `SOURCE_CEILING_BYTES`; `Ok(None)` for any other kind or for an
    /// entry whose `size` exceeds the ceiling.
    pub fn read(&self, repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
        read_blob_source(repo, entry)
    }

    /// A symlink entry's target bytes (D24: WS-2's diff seam); `Ok(None)`
    /// for any other kind.
    pub fn link_target(
        &self,
        repo: &Repository,
        entry: &Entry,
    ) -> Result<Option<Vec<u8>>, GitError> {
        read_blob_link_target(repo, entry)
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
        let bare_entries = read_index_entries(repo)?;

        // Opened once per snapshot (not per entry): `size` comes from the
        // ODB header, never a full blob inflate (perf HIGH). Never
        // `IndexEntry::file_size`, which is the stat size, not the blob's.
        let odb = repo
            .odb()
            .map_err(|err| wrap_git_error("cannot open the object database", &err))?;
        let mut entries = Vec::with_capacity(bare_entries.len());
        for (path, kind, id) in bare_entries {
            let (oid, size) = entry_fields(&odb, kind, id)?;
            entries.push(Entry {
                path,
                kind,
                oid,
                size,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(IndexSnapshot { entries })
    }

    /// Source bytes for a `Regular`/`Executable` entry, bounded by
    /// `SOURCE_CEILING_BYTES`; `Ok(None)` for any other kind or for an
    /// entry whose `size` exceeds the ceiling.
    pub fn read(&self, repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
        read_blob_source(repo, entry)
    }

    /// A symlink entry's target bytes (D24: WS-2's diff seam); `Ok(None)`
    /// for any other kind.
    pub fn link_target(
        &self,
        repo: &Repository,
        entry: &Entry,
    ) -> Result<Option<Vec<u8>>, GitError> {
        read_blob_link_target(repo, entry)
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
        let workdir = worktree_dir(repo)?;

        // No ODB pass here (perf MEDIUM): every size is re-derived from disk
        // below, so an index blob header would be read only to be thrown
        // away.
        let bare_entries = read_index_entries(repo)?;
        let tracked: BTreeSet<RepoPath> = bare_entries
            .iter()
            .map(|(path, _kind, _oid)| path.clone())
            .collect();

        let mut by_path: BTreeMap<RepoPath, Entry> = BTreeMap::new();
        for (path, kind, oid) in bare_entries {
            let full_path = repo_path_to_fs(&workdir, path.as_bytes());
            match fs::symlink_metadata(&full_path) {
                Ok(metadata) => {
                    if let Some(refreshed) = refresh_from_disk(&path, &metadata, kind, Some(oid))? {
                        by_path.insert(path.clone(), refreshed);
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

    /// Source bytes for a `Regular`/`Executable` entry, bounded by
    /// `SOURCE_CEILING_BYTES`: reads at most one byte past the ceiling
    /// (`File::take`) so memory stays bounded even if the file grew on
    /// disk after enumeration, then reports `Ok(None)` if it did.
    pub fn read(&self, repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
        if !matches!(entry.kind, EntryKind::Regular | EntryKind::Executable) {
            return Ok(None);
        }
        if entry.size > SOURCE_CEILING_BYTES {
            return Ok(None);
        }
        let workdir = worktree_dir(repo)?;
        let full_path = repo_path_to_fs(&workdir, entry.path.as_bytes());
        let file = fs::File::open(&full_path).map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!("cannot read worktree file {}: {err}", entry.path.render()),
            )
        })?;
        let mut buf = Vec::new();
        file.take(SOURCE_CEILING_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|err| {
                GitError::new(
                    CODE_SNAPSHOT_UNAVAILABLE,
                    format!("cannot read worktree file {}: {err}", entry.path.render()),
                )
            })?;
        if buf.len() as u64 > SOURCE_CEILING_BYTES {
            return Ok(None);
        }
        Ok(Some(buf))
    }

    /// A symlink entry's target bytes, read fresh from disk (D24: WS-2's
    /// diff seam); `Ok(None)` for any other kind.
    pub fn link_target(
        &self,
        repo: &Repository,
        entry: &Entry,
    ) -> Result<Option<Vec<u8>>, GitError> {
        if entry.kind != EntryKind::Symlink {
            return Ok(None);
        }
        let workdir = worktree_dir(repo)?;
        let full_path = repo_path_to_fs(&workdir, entry.path.as_bytes());
        let target = fs::read_link(&full_path).map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!(
                    "cannot read symlink target for {}: {err}",
                    entry.path.render()
                ),
            )
        })?;
        Ok(Some(os_str_bytes(target.as_os_str())))
    }
}

/// The repository's worktree directory, or `NSD-G101` for a bare
/// repository.
fn worktree_dir(repo: &Repository) -> Result<PathBuf, GitError> {
    Ok(repo
        .workdir()
        .ok_or_else(|| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                "repository has no worktree (bare repository)",
            )
        })?
        .to_path_buf())
}

/// The index's bare rows (path, kind, gitlink/blob oid), with no ODB read:
/// no blob header and no inflate (perf MEDIUM: `WorktreeSnapshot::open`
/// re-derives every size from disk, so reading a header for it here would
/// be wasted work). Fails with `NSD-G101` (D4) when the index holds
/// unresolved merge conflicts (stage 1-3 entries).
fn read_index_entries(repo: &Repository) -> Result<Vec<(RepoPath, EntryKind, Oid)>, GitError> {
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
            // Index entries are never directories: trees are flattened to
            // blob/gitlink rows. An unrecognised mode is skipped.
            None => continue,
        };
        entries.push((RepoPath::from_bytes(index_entry.path), kind, index_entry.id));
    }
    Ok(entries)
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

/// The `oid`/`size` an tree or index blob entry contributes to an `Entry`.
/// `size` comes from the ODB header (no inflate, perf HIGH) for
/// `Regular`/`Executable`; a `Symlink`/`Submodule`/`NestedCheckout` entry
/// carries no source bytes, so its `size` is `0`.
fn entry_fields(odb: &Odb<'_>, kind: EntryKind, id: Oid) -> Result<(Option<Oid>, u64), GitError> {
    match kind {
        EntryKind::Regular | EntryKind::Executable => {
            let (size, _object_type) = odb
                .read_header(id)
                .map_err(|err| wrap_git_error("cannot read blob header", &err))?;
            Ok((Some(id), size as u64))
        }
        EntryKind::Symlink | EntryKind::Submodule | EntryKind::NestedCheckout => Ok((Some(id), 0)),
    }
}

/// Source bytes for a `Regular`/`Executable` `Commit`/`Index` entry,
/// bounded by `SOURCE_CEILING_BYTES`; `Ok(None)` for any other kind or for
/// an entry whose `size` exceeds the ceiling.
fn read_blob_source(repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
    if !matches!(entry.kind, EntryKind::Regular | EntryKind::Executable) {
        return Ok(None);
    }
    if entry.size > SOURCE_CEILING_BYTES {
        return Ok(None);
    }
    let Some(oid) = entry.oid else {
        return Ok(None);
    };
    let blob = repo
        .find_blob(oid)
        .map_err(|err| wrap_git_error("cannot read blob", &err))?;
    Ok(Some(blob.content().to_vec()))
}

/// A `Symlink` entry's target bytes for `Commit`/`Index` (the blob content
/// itself, since Git stores a symlink's target as its blob); `Ok(None)`
/// for any other kind.
fn read_blob_link_target(repo: &Repository, entry: &Entry) -> Result<Option<Vec<u8>>, GitError> {
    if entry.kind != EntryKind::Symlink {
        return Ok(None);
    }
    let Some(oid) = entry.oid else {
        return Ok(None);
    };
    let blob = repo
        .find_blob(oid)
        .map_err(|err| wrap_git_error("cannot read blob", &err))?;
    Ok(Some(blob.content().to_vec()))
}

/// Recursively walks `tree`, accumulating every blob/symlink/gitlink entry
/// under `prefix` (raw bytes, `/`-joined) into `entries`. `odb` is opened
/// once by the caller and passed down, not reopened per entry.
fn walk_tree(
    repo: &Repository,
    odb: &Odb<'_>,
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
            walk_tree(repo, odb, &subtree, &path_bytes, entries)?;
            continue;
        }
        let Some(kind) = entry_kind(mode) else {
            continue; // Unrecognised mode: skip (git itself only writes the five above).
        };
        let (oid, size) = entry_fields(odb, kind, tree_entry.id())?;
        entries.push(Entry {
            path: RepoPath::from_bytes(path_bytes),
            kind,
            oid,
            size,
        });
    }
    Ok(())
}

/// Joins a repository-relative byte path onto `workdir` for a filesystem
/// access. A tracked path can carry non-UTF-8 bytes (an attacker-controlled
/// blob path, or any byte a filesystem other than the one running this
/// check will accept), and a lossy conversion would silently alias it to
/// whatever real file happens to share the escaped name. On unix, the path
/// is rebuilt from its exact bytes via `OsStr::from_bytes`, so no byte is
/// ever substituted. Only on a platform without a byte-oriented `OsStr`
/// does this fall back to a lossy conversion.
#[cfg(unix)]
fn repo_path_to_fs(workdir: &Path, path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    workdir.join(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
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
    metadata: &fs::Metadata,
    original_kind: EntryKind,
    original_oid: Option<Oid>,
) -> Result<Option<Entry>, GitError> {
    if metadata.is_dir() {
        if original_kind == EntryKind::Submodule {
            // D24: keep the index's gitlink OID (WS-2's diff seam needs
            // it), rather than dropping it to `None`.
            return Ok(Some(Entry {
                path: path.clone(),
                kind: EntryKind::Submodule,
                oid: original_oid,
                size: 0,
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
        }));
    }
    let kind = if is_executable(metadata) {
        EntryKind::Executable
    } else {
        EntryKind::Regular
    };
    Ok(Some(Entry {
        path: path.clone(),
        kind,
        oid: None,
        size: metadata.len(),
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
                },
            );
            continue;
        }
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
                size: metadata.len(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn repo_path_to_fs_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;

        let mut invalid_name = b"a".to_vec();
        invalid_name.push(0xFF);
        invalid_name.extend_from_slice(b".ts");

        let joined = repo_path_to_fs(Path::new("/w"), &invalid_name);

        let mut expected = b"/w/a".to_vec();
        expected.push(0xFF);
        expected.extend_from_slice(b".ts");
        assert_eq!(joined.as_os_str().as_bytes(), expected.as_slice());
    }
}

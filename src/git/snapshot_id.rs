//! Content-addressed snapshot IDs (M2-4): one `SnapshotId` per `Commit`/
//! `Index`/`Worktree` snapshot, built with `hashing::Digest` over every
//! entry in the snapshot's own raw-path-byte order. Each entry contributes
//! its path bytes, a one-byte kind tag, and (except for `NestedCheckout`/
//! `Special`, which have none) its content object id, so two snapshots with
//! identical content in any mode share the same ID (`nsd-plan-implementation
//! .md:78`: "hashes as algorithm-prefixed lowercase hex"). Computing an ID
//! never writes to the Git object database (D2).

use std::path::Path;

use git2::{ObjectType, Oid, Repository};

use crate::hashing::Digest;

use super::diff::worktree_blob_oid;
use super::snapshot::{
    worktree_dir, CommitSnapshot, Entry, EntryKind, IndexSnapshot, WorktreeSnapshot,
};
use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

/// `Digest`'s domain separator for this module (do-not-reuse:
/// `src/profile.rs::FINGERPRINT_FAMILY_PREFIX` is the measurement-profile
/// domain, not snapshot content).
const SNAPSHOT_ID_FAMILY_PREFIX: &str = "git-snapshot-id";

/// A stable one-byte discriminator per `EntryKind`, hashed alongside every
/// entry's path and content id so a mode change (Regular <-> Executable) or
/// a kind change at the same path moves the ID even when the object id
/// itself is unchanged.
fn entry_kind_tag(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Regular => 0,
        EntryKind::Executable => 1,
        EntryKind::Symlink => 2,
        EntryKind::Submodule => 3,
        EntryKind::NestedCheckout => 4,
        EntryKind::Special => 5,
    }
}

/// A commit, index or worktree snapshot's content-addressed ID, displayed as
/// `blake3:<32 lowercase hex>`. Never a commit/tree oid (do-not-reuse: an
/// index or worktree snapshot has none, and IDs must agree across modes for
/// equal content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotId(u128);

impl SnapshotId {
    /// The commit `snapshot`'s ID, built from its entries' stored oids.
    pub fn of_commit(snapshot: &CommitSnapshot) -> SnapshotId {
        SnapshotId(hash_stored_entries(&snapshot.entries))
    }

    /// The index `snapshot`'s ID (`--staged`'s candidate), built from its
    /// entries' stored oids -- never the worktree's on-disk bytes.
    pub fn of_index(snapshot: &IndexSnapshot) -> SnapshotId {
        SnapshotId(hash_stored_entries(&snapshot.entries))
    }

    /// The worktree `snapshot`'s ID. A Regular/Executable entry's object id
    /// comes from `worktree_blob_oid` (real content, or, over
    /// `SOURCE_CEILING_BYTES`, a raw fd hash of the on-disk bytes -- either
    /// way, no ODB write); a Symlink's from hashing its on-disk target as a
    /// blob, exactly as `diff_commit_to_worktree` does; a Submodule's from
    /// its kept gitlink oid. Fails with `NSD-G101` only when reading an
    /// entry's bytes off disk fails.
    pub fn of_worktree(
        repo: &Repository,
        snapshot: &WorktreeSnapshot,
    ) -> Result<SnapshotId, GitError> {
        let workdir = worktree_dir(repo)?;
        let mut digest = Digest::new(SNAPSHOT_ID_FAMILY_PREFIX);
        for entry in &snapshot.entries {
            let oid = worktree_entry_oid(repo, &workdir, snapshot, entry)?;
            push_entry(&mut digest, entry, oid);
        }
        Ok(SnapshotId(digest.finish()))
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "blake3:{:032x}", self.0)
    }
}

/// `Commit`/`Index` entries: an entry's stored oid, except `NestedCheckout`/
/// `Special` (unreachable from a Git mode, `entry_fields`), which contribute
/// no object id.
fn hash_stored_entries(entries: &[Entry]) -> u128 {
    let mut digest = Digest::new(SNAPSHOT_ID_FAMILY_PREFIX);
    for entry in entries {
        let oid = match entry.kind {
            EntryKind::NestedCheckout | EntryKind::Special => None,
            _ => entry.oid,
        };
        push_entry(&mut digest, entry, oid);
    }
    digest.finish()
}

/// Feeds one entry's path bytes, kind tag and (if any) object id bytes into
/// `digest`, in that fixed order.
fn push_entry(digest: &mut Digest, entry: &Entry, oid: Option<Oid>) {
    digest.push(entry.path.as_bytes());
    digest.push(&[entry_kind_tag(entry.kind)]);
    if let Some(oid) = oid {
        digest.push(oid.as_bytes());
    }
}

/// The worktree entry's content object id (`None` for `NestedCheckout`/
/// `Special`, which contribute no id).
fn worktree_entry_oid(
    repo: &Repository,
    workdir: &Path,
    snapshot: &WorktreeSnapshot,
    entry: &Entry,
) -> Result<Option<Oid>, GitError> {
    match entry.kind {
        EntryKind::Regular | EntryKind::Executable => {
            Ok(Some(worktree_blob_oid(repo, workdir, snapshot, entry)?))
        }
        EntryKind::Symlink => {
            let target = snapshot.link_target(repo, entry)?.ok_or_else(|| {
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
            Ok(Some(oid))
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
            Ok(Some(oid))
        }
        EntryKind::NestedCheckout | EntryKind::Special => Ok(None),
    }
}

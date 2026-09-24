//! Shared test fixture helpers (D22): every `tests/*.rs` suite that
//! declares `mod common;` calls both of these, and nothing else lives
//! here — each suite's own helpers (index-only entries, worktree files,
//! shallow clones, conflicted indexes, buffers) live in that one suite.

use std::collections::BTreeMap;

use git2::{Commit, Oid, Repository, Signature};
use tempfile::TempDir;

/// A gitlink (submodule) tree/index entry's target is a commit, not a
/// blob (D22's "content bytes" for mode `0o160000` are read as exactly the
/// 20 raw bytes of that target commit's id — libgit2's `TreeBuilder::insert`
/// does not require the target to actually exist).
const MODE_SUBMODULE: i32 = 0o160000;
const MODE_TREE: i32 = 0o040000;

/// One fixture entry: a repository-relative path, a Git file mode, and raw
/// content bytes (or, for a gitlink, the 20 raw bytes of the target commit
/// id). Factored out of a plain tuple per `clippy::type_complexity`.
type FixtureEntry = (Vec<u8>, i32, Vec<u8>);

/// Creates a fresh temporary repository, returning the guard that keeps its
/// directory alive alongside the opened `git2::Repository`.
pub fn init_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("create a temp dir for the test repository");
    let repo = Repository::init(dir.path()).expect("init the test repository");
    (dir, repo)
}

/// Commits `entries` (raw byte path, file mode, content bytes) to `HEAD`
/// through git2's index/tree APIs only (D23: never written to disk, since
/// APFS rejects non-UTF-8 file names), returning the resulting commit id.
/// A path with a `/` is split into nested trees. `HEAD`'s current commit,
/// if any, becomes the new commit's sole parent.
pub fn commit_entries(repo: &Repository, entries: &[FixtureEntry]) -> Oid {
    let tree_oid = build_tree(repo, entries);
    let tree = repo.find_tree(tree_oid).expect("find the built tree");
    let signature =
        Signature::now("nsd test fixture", "fixture@example.invalid").expect("build a signature");
    let parent_commit: Option<Commit<'_>> = repo
        .head()
        .ok()
        .and_then(|head_ref| head_ref.peel_to_commit().ok());
    let parents: Vec<&Commit<'_>> = parent_commit.iter().collect();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "test fixture commit",
        &tree,
        &parents,
    )
    .expect("commit the test fixture tree")
}

fn build_tree(repo: &Repository, entries: &[FixtureEntry]) -> Oid {
    let mut builder = repo.treebuilder(None).expect("create a tree builder");
    let mut subdirs: BTreeMap<Vec<u8>, Vec<FixtureEntry>> = BTreeMap::new();

    for (path, mode, content) in entries {
        match split_first_component(path) {
            Some((head, tail)) => {
                subdirs.entry(head.to_vec()).or_default().push((
                    tail.to_vec(),
                    *mode,
                    content.clone(),
                ));
            }
            None => {
                let oid = if *mode == MODE_SUBMODULE {
                    Oid::from_bytes(content)
                        .expect("submodule fixture content is 20 raw gitlink-target bytes")
                } else {
                    repo.blob(content).expect("write a blob")
                };
                builder
                    .insert(path.as_slice(), oid, *mode)
                    .expect("insert a tree entry");
            }
        }
    }

    for (name, child_entries) in subdirs {
        let subtree_oid = build_tree(repo, &child_entries);
        builder
            .insert(name.as_slice(), subtree_oid, MODE_TREE)
            .expect("insert a subtree entry");
    }

    builder.write().expect("write the tree")
}

fn split_first_component(path: &[u8]) -> Option<(&[u8], &[u8])> {
    path.iter()
        .position(|&byte| byte == b'/')
        .map(|pos| (&path[..pos], &path[pos + 1..]))
}

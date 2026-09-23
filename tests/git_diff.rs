//! WS-2 acceptance: the central file-level Change listing (D8 renames at
//! `RENAME_THRESHOLD`, A6's two fixtures, commit/index/worktree diff modes,
//! D24's mempack seam, D10's sort) and the `map_lines` line-mapping
//! primitive (D9's forced text mode, D11's 1-based lines).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use git2::{IndexEntry, IndexTime, Oid, Repository};
use tempfile::TempDir;

use nsd::git::diff::{self, Change};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::{EntryKind, WorktreeSnapshot, SOURCE_CEILING_BYTES};

const MODE_REGULAR: i32 = 0o100644;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

#[test]
fn rename_threshold_is_fifty() {
    assert_eq!(diff::RENAME_THRESHOLD, 50);
}

#[test]
fn clear_rename_detected() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(
            b"src/Old.java".to_vec(),
            MODE_REGULAR,
            numbered_lines(40, None),
        )],
    );
    sync_index_to_commit(&repo, base_oid);

    let mut index = repo.index().expect("open index");
    index
        .remove_path(Path::new("src/Old.java"))
        .expect("git mv: remove the old path from the index");
    index.write().expect("write index after removal");
    stage_bytes(
        &repo,
        b"src/New.java",
        MODE_REGULAR,
        &numbered_lines(40, Some((20, "line 20 EDITED"))),
    );

    let changes = diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");

    assert_eq!(changes.len(), 1, "expected exactly one change: {changes:?}");
    match &changes[0] {
        Change::Renamed { from, to, kind } => {
            assert_eq!(from.as_bytes(), b"src/Old.java");
            assert_eq!(to.as_bytes(), b"src/New.java");
            assert_eq!(*kind, EntryKind::Regular);
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
}

#[test]
fn clear_non_rename_is_delete_plus_add() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"src/A.ts".to_vec(), MODE_REGULAR, filler_lines(b'A', 40))],
    );
    sync_index_to_commit(&repo, base_oid);

    let mut index = repo.index().expect("open index");
    index
        .remove_path(Path::new("src/A.ts"))
        .expect("stage the delete of src/A.ts");
    index.write().expect("write index after removal");
    stage_bytes(&repo, b"src/B.ts", MODE_REGULAR, &filler_lines(b'Z', 40));

    let changes = diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");

    assert_eq!(
        changes.len(),
        2,
        "expected a delete and an unrelated add, not a rename: {changes:?}"
    );
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Deleted { path, .. } if path.as_bytes() == b"src/A.ts")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"src/B.ts")));
}

#[test]
fn staged_add_delete_modify_statuses() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"base-a\n".to_vec()),
            (b"b.ts".to_vec(), MODE_REGULAR, b"base-b\n".to_vec()),
            (b"c.ts".to_vec(), MODE_REGULAR, b"base-c\n".to_vec()),
        ],
    );
    sync_index_to_commit(&repo, base_oid);

    let mut index = repo.index().expect("open index");
    index
        .remove_path(Path::new("b.ts"))
        .expect("stage the delete of b.ts");
    index.write().expect("write index after removal");
    stage_bytes(&repo, b"c.ts", MODE_REGULAR, b"modified-c\n");
    stage_bytes(&repo, b"d.ts", MODE_REGULAR, b"new-d\n");

    let changes = diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");

    assert_eq!(
        changes.len(),
        3,
        "unchanged a.ts must not appear: {changes:?}"
    );
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"d.ts")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Deleted { path, .. } if path.as_bytes() == b"b.ts")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Modified { path, .. } if path.as_bytes() == b"c.ts")));
}

#[test]
fn unborn_empty_tree_to_index_all_added() {
    let (_dir, repo) = common::init_repo();
    stage_bytes(&repo, b"a.ts", MODE_REGULAR, b"a\n");
    stage_bytes(&repo, b"b.ts", MODE_REGULAR, b"b\n");

    let changes = diff::diff_commit_to_index(&repo, None).expect("diff empty tree to index");

    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes.iter().all(|c| matches!(c, Change::Added { .. })));
}

#[test]
fn worktree_diff_includes_untracked_and_isolates_dirty_state() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"tracked.ts".to_vec(), MODE_REGULAR, b"base\n".to_vec())],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::write(dir.path().join("tracked.ts"), b"changed\n").expect("edit tracked.ts unstaged");
    std::fs::write(dir.path().join("untracked.ts"), b"new\n").expect("write untracked.ts");

    let before = count_odb_objects(&repo);
    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let worktree_changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");
    let after = count_odb_objects(&repo);

    assert_eq!(before, after, "D2/D24: the on-disk ODB must be untouched");
    assert!(worktree_changes
        .iter()
        .any(|c| matches!(c, Change::Modified { path, .. } if path.as_bytes() == b"tracked.ts")));
    assert!(worktree_changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"untracked.ts")));

    let index_changes =
        diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");
    assert!(
        index_changes.is_empty(),
        "commit-to-index must not see unstaged worktree edits: {index_changes:?}"
    );
}

#[test]
fn symlink_and_submodule_changes_are_typed() {
    let (_dir, repo) = common::init_repo();
    let base_gitlink = [0xAAu8; 20];
    let base_oid = common::commit_entries(
        &repo,
        &[(
            b"vendor/lib".to_vec(),
            MODE_SUBMODULE,
            base_gitlink.to_vec(),
        )],
    );
    sync_index_to_commit(&repo, base_oid);

    let new_gitlink = [0xBBu8; 20];
    stage_gitlink(&repo, b"vendor/lib", &new_gitlink);
    stage_bytes(&repo, b"link.ts", MODE_SYMLINK, b"target.ts");

    let changes = diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");

    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes.iter().any(|c| matches!(
        c,
        Change::Modified {
            path,
            kind: EntryKind::Submodule
        } if path.as_bytes() == b"vendor/lib"
    )));
    assert!(changes.iter().any(|c| matches!(
        c,
        Change::Added {
            path,
            kind: EntryKind::Symlink
        } if path.as_bytes() == b"link.ts"
    )));
}

#[test]
fn worktree_over_ceiling_unchanged_file_is_not_reported() {
    let (dir, repo) = common::init_repo();
    let over_ceiling = vec![b'a'; SOURCE_CEILING_BYTES as usize + 1];
    let base_oid = common::commit_entries(
        &repo,
        &[(b"big.js".to_vec(), MODE_REGULAR, over_ceiling.clone())],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::write(dir.path().join("big.js"), &over_ceiling)
        .expect("rewrite big.js with identical over-ceiling content");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert!(
        changes.is_empty(),
        "an unchanged tracked over-ceiling file must not appear: {changes:?}"
    );
}

#[test]
fn worktree_over_ceiling_file_over_empty_base_is_modified() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(&repo, &[(b"src/x.ts".to_vec(), MODE_REGULAR, Vec::new())]);
    sync_index_to_commit(&repo, base_oid);
    std::fs::create_dir_all(dir.path().join("src")).expect("create src/ directory");
    let over_ceiling = vec![b'b'; SOURCE_CEILING_BYTES as usize + 1];
    std::fs::write(dir.path().join("src/x.ts"), &over_ceiling)
        .expect("write over-ceiling content over an empty base file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(matches!(
        &changes[0],
        Change::Modified {
            path,
            kind: EntryKind::Regular
        } if path.as_bytes() == b"src/x.ts"
    ));
}

#[test]
fn worktree_over_ceiling_add_is_not_a_rename_of_an_empty_file() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(&repo, &[(b"empty.ts".to_vec(), MODE_REGULAR, Vec::new())]);
    sync_index_to_commit(&repo, base_oid);
    // `commit_entries` never writes to disk (D23), so `empty.ts` is already
    // absent from the worktree — an on-disk deletion, with nothing to
    // remove.
    let over_ceiling = vec![b'c'; SOURCE_CEILING_BYTES as usize + 1];
    std::fs::write(dir.path().join("big.js"), &over_ceiling)
        .expect("write an unrelated untracked over-ceiling file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(changes.len(), 2, "expected a delete and an unrelated add, not a rename: {changes:?}");
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Deleted { path, .. } if path.as_bytes() == b"empty.ts")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"big.js")));
}

#[test]
fn changes_sorted_by_raw_path_bytes() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"m.ts".to_vec(), MODE_REGULAR, b"base\n".to_vec())],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::write(dir.path().join("m.ts"), b"changed\n").expect("edit m.ts unstaged");

    // Written out of byte order, and merged in after the libgit2-produced
    // deltas (D24), to prove the final sort is ours, not an artifact of
    // libgit2's own already-sorted delta order.
    write_nested_checkout(&dir, "z-nested");
    write_nested_checkout(&dir, "a-nested");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    let paths: Vec<Vec<u8>> = changes.iter().map(|c| change_path(c).to_vec()).collect();
    assert_eq!(
        paths,
        vec![b"a-nested".to_vec(), b"m.ts".to_vec(), b"z-nested".to_vec(),]
    );
}

#[test]
fn line_map_insertion_maps_unchanged_lines() {
    let base: &[u8] = b"a\nb\nc\n";
    let candidate: &[u8] = b"a\nX\nb\nc\n";

    let map = diff::map_lines(base, candidate).expect("map lines");

    assert_eq!(
        map.base_to_candidate,
        BTreeMap::from([(1, 1), (2, 3), (3, 4)])
    );
    assert_eq!(map.added_candidate_lines, BTreeSet::from([2]));
    assert_eq!(map.deleted_base_lines, BTreeSet::new());
}

#[test]
fn line_map_pure_shift() {
    let base: &[u8] = b"a\nb\nc\n";
    let candidate: &[u8] = b"x\ny\nz\na\nb\nc\n";

    let map = diff::map_lines(base, candidate).expect("map lines");

    assert_eq!(
        map.base_to_candidate,
        BTreeMap::from([(1, 4), (2, 5), (3, 6)])
    );
    assert_eq!(map.added_candidate_lines, BTreeSet::from([1, 2, 3]));
    assert_eq!(map.deleted_base_lines, BTreeSet::new());
}

#[test]
fn line_map_no_trailing_newline() {
    let base: &[u8] = b"a\nb\nc";
    let candidate: &[u8] = b"a\nb\nX";

    let map = diff::map_lines(base, candidate).expect("map lines");

    assert_eq!(map.base_to_candidate, BTreeMap::from([(1, 1), (2, 2)]));
    assert_eq!(map.deleted_base_lines, BTreeSet::from([3]));
    assert_eq!(map.added_candidate_lines, BTreeSet::from([3]));
}

#[test]
fn line_map_forces_text_on_nul_bytes() {
    let base: &[u8] = b"a\n\0b\nc\n";
    let candidate: &[u8] = b"a\n\0b\nX\n";

    let map = diff::map_lines(base, candidate).expect("map lines");

    assert_eq!(
        map.base_to_candidate,
        BTreeMap::from([(1, 1), (2, 2)]),
        "a NUL-containing buffer must still yield line hunks, not \"binary, no lines\""
    );
    assert_eq!(map.deleted_base_lines, BTreeSet::from([3]));
    assert_eq!(map.added_candidate_lines, BTreeSet::from([3]));
}

#[test]
fn line_map_identical_buffers_is_identity() {
    let buffer: &[u8] = b"a\nb\nc\n";

    let map = diff::map_lines(buffer, buffer).expect("map lines");

    assert_eq!(
        map.base_to_candidate,
        BTreeMap::from([(1, 1), (2, 2), (3, 3)])
    );
    assert!(map.added_candidate_lines.is_empty());
    assert!(map.deleted_base_lines.is_empty());
}

fn numbered_lines(count: usize, edit: Option<(usize, &str)>) -> Vec<u8> {
    let mut buf = String::new();
    for i in 0..count {
        match edit {
            Some((idx, replacement)) if idx == i => buf.push_str(replacement),
            _ => buf.push_str(&format!("line {i}")),
        }
        buf.push('\n');
    }
    buf.into_bytes()
}

/// Forty lines with no content in common with the same call using a
/// different `byte`, so libgit2's chunk-hash similarity score is nowhere
/// near `RENAME_THRESHOLD` (A6's "clear non-rename fixture").
fn filler_lines(byte: u8, count: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    for _ in 0..count {
        buf.extend(std::iter::repeat_n(byte, 20));
        buf.push(b'\n');
    }
    buf
}

fn write_nested_checkout(dir: &TempDir, name: &str) {
    let nested_dir = dir.path().join(name);
    std::fs::create_dir_all(&nested_dir).expect("create nested checkout directory");
    std::fs::write(nested_dir.join(".git"), b"gitdir: /elsewhere\n")
        .expect("write nested .git marker");
}

fn change_path(change: &Change) -> &[u8] {
    match change {
        Change::Added { path, .. }
        | Change::Deleted { path, .. }
        | Change::Modified { path, .. }
        | Change::Typechange { path, .. } => path.as_bytes(),
        Change::Renamed { to, .. } => to.as_bytes(),
    }
}

fn stage_bytes(repo: &Repository, path: &[u8], mode: i32, content: &[u8]) {
    let mut index = repo.index().expect("open index");
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: mode as u32,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::ZERO_SHA1,
        flags: 0,
        flags_extended: 0,
        path: path.to_vec(),
    };
    index.add_frombuffer(&entry, content).expect("stage buffer");
    index.write().expect("write index");
}

fn stage_gitlink(repo: &Repository, path: &[u8], target: &[u8; 20]) {
    let mut index = repo.index().expect("open index");
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: MODE_SUBMODULE as u32,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::from_bytes(target).expect("build the gitlink oid"),
        flags: 0,
        flags_extended: 0,
        path: path.to_vec(),
    };
    index.add(&entry).expect("stage gitlink");
    index.write().expect("write index");
}

fn sync_index_to_commit(repo: &Repository, commit_oid: Oid) {
    let commit = repo.find_commit(commit_oid).expect("find commit");
    let tree = commit.tree().expect("commit tree");
    let mut index = repo.index().expect("open index");
    index.read_tree(&tree).expect("read tree into index");
    index.write().expect("write index");
}

fn count_odb_objects(repo: &Repository) -> usize {
    let odb = repo.odb().expect("open odb");
    let mut count = 0usize;
    odb.foreach(|_oid| {
        count += 1;
        true
    })
    .expect("walk odb");
    count
}

//! WS-4 acceptance (M2-4): `SnapshotId` agrees across a clean checkout's
//! commit/index/worktree modes, moves on a content/mode/path/kind change,
//! is frozen for two literal snapshots, and never writes to the ODB.

mod common;

use git2::{IndexEntry, IndexTime, Oid, Repository};

use nsd::git::snapshot::{CommitSnapshot, IndexSnapshot, WorktreeSnapshot, SOURCE_CEILING_BYTES};
use nsd::git::snapshot_id::SnapshotId;

const MODE_REGULAR: i32 = 0o100644;
const MODE_EXECUTABLE: i32 = 0o100755;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

#[test]
fn test_clean_checkout_ids_agree_across_modes() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"one\n".to_vec()),
            (b"run.sh".to_vec(), MODE_EXECUTABLE, b"#!/bin/sh\n".to_vec()),
        ],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("a.ts"), b"one\n").expect("write a.ts");
    std::fs::write(dir.path().join("run.sh"), b"#!/bin/sh\n").expect("write run.sh");
    chmod_executable(&dir.path().join("run.sh"));

    let commit_id =
        SnapshotId::of_commit(&CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot"));
    let index_id = SnapshotId::of_index(&IndexSnapshot::open(&repo).expect("open index snapshot"));
    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let worktree_id = SnapshotId::of_worktree(&repo, &worktree).expect("compute worktree id");

    assert_eq!(
        commit_id, index_id,
        "a clean checkout's commit and index IDs must agree"
    );
    assert_eq!(
        index_id, worktree_id,
        "a clean checkout's index and worktree IDs must agree"
    );

    // Editing one byte of one worktree file changes only the worktree ID.
    std::fs::write(dir.path().join("a.ts"), b"ONE\n").expect("edit a.ts on disk");
    let dirty_worktree = WorktreeSnapshot::open(&repo).expect("open dirty worktree snapshot");
    let dirty_worktree_id =
        SnapshotId::of_worktree(&repo, &dirty_worktree).expect("compute dirty worktree id");
    assert_ne!(
        worktree_id, dirty_worktree_id,
        "a one-byte worktree edit must change the worktree ID"
    );
    let commit_id_after = SnapshotId::of_commit(
        &CommitSnapshot::head_or_empty(&repo).expect("re-open commit snapshot"),
    );
    let index_id_after =
        SnapshotId::of_index(&IndexSnapshot::open(&repo).expect("re-open index snapshot"));
    assert_eq!(
        commit_id, commit_id_after,
        "a worktree-only edit must not move the commit ID"
    );
    assert_eq!(
        index_id, index_id_after,
        "a worktree-only edit must not move the index ID"
    );
}

#[test]
#[cfg(unix)]
fn test_clean_checkout_ids_agree_across_modes_with_symlink_and_gitlink() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"one\n".to_vec()),
            (b"link".to_vec(), MODE_SYMLINK, b"a.ts".to_vec()),
            (
                b"vendor/lib".to_vec(),
                MODE_SUBMODULE,
                [0xCCu8; 20].to_vec(),
            ),
        ],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("a.ts"), b"one\n").expect("write a.ts");
    std::os::unix::fs::symlink("a.ts", dir.path().join("link"))
        .expect("create the on-disk symlink");
    std::fs::create_dir_all(dir.path().join("vendor").join("lib"))
        .expect("create the submodule directory");

    let commit_id =
        SnapshotId::of_commit(&CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot"));
    let index_id = SnapshotId::of_index(&IndexSnapshot::open(&repo).expect("open index snapshot"));
    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let worktree_id = SnapshotId::of_worktree(&repo, &worktree).expect("compute worktree id");

    assert_eq!(
        commit_id, index_id,
        "commit and index IDs must agree with a symlink and a gitlink present"
    );
    assert_eq!(
        index_id, worktree_id,
        "index and worktree IDs must agree with a symlink and a gitlink present"
    );

    // Re-point the on-disk link; the worktree ID must move, the commit ID
    // must not.
    std::fs::remove_file(dir.path().join("link")).expect("remove the old on-disk symlink");
    std::os::unix::fs::symlink("b.ts", dir.path().join("link"))
        .expect("re-point the on-disk symlink");
    let repointed_worktree =
        WorktreeSnapshot::open(&repo).expect("open re-pointed worktree snapshot");
    let repointed_worktree_id =
        SnapshotId::of_worktree(&repo, &repointed_worktree).expect("compute re-pointed id");
    assert_ne!(
        worktree_id, repointed_worktree_id,
        "re-pointing the on-disk symlink target must change the worktree ID"
    );
    let commit_id_after = SnapshotId::of_commit(
        &CommitSnapshot::head_or_empty(&repo).expect("re-open commit snapshot"),
    );
    assert_eq!(
        commit_id, commit_id_after,
        "re-pointing the on-disk symlink must not move the commit ID"
    );
}

#[test]
fn test_id_is_deterministic_and_checkout_root_independent() {
    let entries: &[(Vec<u8>, i32, Vec<u8>)] = &[
        (
            b"src/a.ts".to_vec(),
            MODE_REGULAR,
            b"content one\n".to_vec(),
        ),
        (
            b"src/b.ts".to_vec(),
            MODE_REGULAR,
            b"content two\n".to_vec(),
        ),
    ];

    let (dir_a, repo_a) = common::init_repo();
    let commit_a = common::commit_entries(&repo_a, entries);
    sync_index_to_commit(&repo_a, commit_a);
    write_worktree_files(dir_a.path(), entries);

    let (dir_b, repo_b) = common::init_repo();
    let commit_b = common::commit_entries(&repo_b, entries);
    sync_index_to_commit(&repo_b, commit_b);
    write_worktree_files(dir_b.path(), entries);

    let worktree_a = WorktreeSnapshot::open(&repo_a).expect("open worktree snapshot a");
    let worktree_b = WorktreeSnapshot::open(&repo_b).expect("open worktree snapshot b");
    let id_a = SnapshotId::of_worktree(&repo_a, &worktree_a).expect("compute id a");
    let id_b = SnapshotId::of_worktree(&repo_b, &worktree_b).expect("compute id b");

    assert_eq!(
        id_a, id_b,
        "the same content checked out into two different temp dirs must give equal IDs: {} vs {}",
        id_a, id_b
    );
}

#[test]
fn test_one_byte_change_changes_the_id() {
    let (_dir, repo) = common::init_repo();
    let commit_one = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"one\n".to_vec())],
    );
    let commit_two = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"one!\n".to_vec())],
    );

    let id_one =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, commit_one).expect("open commit one"));
    let id_two =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, commit_two).expect("open commit two"));

    assert_ne!(
        id_one, id_two,
        "a one-byte content change must change the ID"
    );
}

#[test]
fn test_mode_change_changes_the_id() {
    let (_dir, repo) = common::init_repo();
    let regular = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"same\n".to_vec())],
    );
    let executable = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_EXECUTABLE, b"same\n".to_vec())],
    );

    let regular_id =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, regular).expect("open regular commit"));
    let executable_id = SnapshotId::of_commit(
        &CommitSnapshot::at(&repo, executable).expect("open executable commit"),
    );

    assert_ne!(
        regular_id, executable_id,
        "Regular vs Executable at the same path/content must change the ID"
    );
}

#[test]
fn test_path_rename_changes_the_id() {
    let (_dir, repo) = common::init_repo();
    let before = common::commit_entries(
        &repo,
        &[(b"old.ts".to_vec(), MODE_REGULAR, b"same\n".to_vec())],
    );
    let after = common::commit_entries(
        &repo,
        &[(b"new.ts".to_vec(), MODE_REGULAR, b"same\n".to_vec())],
    );

    let before_id =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, before).expect("open before commit"));
    let after_id =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, after).expect("open after commit"));

    assert_ne!(
        before_id, after_id,
        "the same content at a different path must change the ID"
    );
}

#[test]
fn test_symlink_target_and_gitlink_change_the_id() {
    let (_dir, repo) = common::init_repo();
    let link_a =
        common::commit_entries(&repo, &[(b"link".to_vec(), MODE_SYMLINK, b"a.ts".to_vec())]);
    let link_b =
        common::commit_entries(&repo, &[(b"link".to_vec(), MODE_SYMLINK, b"b.ts".to_vec())]);
    let link_id_a =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, link_a).expect("open link commit a"));
    let link_id_b =
        SnapshotId::of_commit(&CommitSnapshot::at(&repo, link_b).expect("open link commit b"));
    assert_ne!(
        link_id_a, link_id_b,
        "a different symlink target must change the ID"
    );

    let gitlink_a = common::commit_entries(
        &repo,
        &[(
            b"vendor/lib".to_vec(),
            MODE_SUBMODULE,
            [0xAAu8; 20].to_vec(),
        )],
    );
    let gitlink_b = common::commit_entries(
        &repo,
        &[(
            b"vendor/lib".to_vec(),
            MODE_SUBMODULE,
            [0xBBu8; 20].to_vec(),
        )],
    );
    let gitlink_id_a = SnapshotId::of_commit(
        &CommitSnapshot::at(&repo, gitlink_a).expect("open gitlink commit a"),
    );
    let gitlink_id_b = SnapshotId::of_commit(
        &CommitSnapshot::at(&repo, gitlink_b).expect("open gitlink commit b"),
    );
    assert_ne!(
        gitlink_id_a, gitlink_id_b,
        "a different gitlink target must change the ID"
    );
}

#[test]
fn test_empty_tree_id_is_frozen() {
    let (_dir, repo) = common::init_repo();
    let empty = CommitSnapshot::head_or_empty(&repo).expect("resolve the empty-tree base");
    assert!(empty.entries.is_empty(), "an unborn HEAD is the empty tree");

    let id = SnapshotId::of_commit(&empty);
    // Pinned: a future change to the hash input framing or `HASH_VERSION`
    // must fail loudly here rather than silently moving every snapshot ID.
    assert_eq!(id.to_string(), "blake3:d6f756a09ed84ef1be2c0ec61901a915");
}

#[test]
fn test_small_snapshot_id_is_frozen() {
    let (_dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"one\n".to_vec()),
            (b"b.ts".to_vec(), MODE_EXECUTABLE, b"two\n".to_vec()),
        ],
    );
    let snapshot = CommitSnapshot::at(&repo, commit_oid).expect("open the two-file commit");
    let id = SnapshotId::of_commit(&snapshot);

    let rendered = id.to_string();
    assert!(
        regex_matches_blake3_hex(&rendered),
        "{rendered} must match ^blake3:[0-9a-f]{{32}}$"
    );
    // Pinned: same rationale as `test_empty_tree_id_is_frozen`.
    assert_eq!(rendered, "blake3:08707c83c277aa8cea3769e1f43dfb9f");
}

#[test]
fn test_over_ceiling_worktree_edit_changes_the_id() {
    let (dir, repo) = common::init_repo();
    let over_ceiling = vec![b'a'; SOURCE_CEILING_BYTES as usize + 1];
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"big.js".to_vec(), MODE_REGULAR, over_ceiling.clone())],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("big.js"), &over_ceiling).expect("write over-ceiling file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let id_before = SnapshotId::of_worktree(&repo, &worktree).expect("compute id before the edit");

    let mut edited = over_ceiling.clone();
    edited[0] = b'z';
    std::fs::write(dir.path().join("big.js"), &edited).expect("edit the over-ceiling file");
    let dirty_worktree = WorktreeSnapshot::open(&repo).expect("open dirty worktree snapshot");
    let id_after =
        SnapshotId::of_worktree(&repo, &dirty_worktree).expect("compute id after the edit");

    assert_ne!(
        id_before, id_after,
        "a one-byte edit to an over-ceiling worktree file must change the ID"
    );
}

#[test]
fn test_worktree_id_writes_nothing_to_the_odb() {
    let (dir, repo) = common::init_repo();
    let over_ceiling = vec![b'b'; SOURCE_CEILING_BYTES as usize + 1];
    let gitlink_target = [0xCCu8; 20];
    let commit_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"small\n".to_vec()),
            (b"big.js".to_vec(), MODE_REGULAR, over_ceiling.clone()),
            (b"link".to_vec(), MODE_SYMLINK, b"a.ts".to_vec()),
            (
                b"vendor/lib".to_vec(),
                MODE_SUBMODULE,
                gitlink_target.to_vec(),
            ),
        ],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("a.ts"), b"small\n").expect("write a.ts");
    std::fs::write(dir.path().join("big.js"), &over_ceiling).expect("write big.js");
    std::os::unix::fs::symlink("a.ts", dir.path().join("link")).expect("write link");
    std::fs::create_dir_all(dir.path().join("vendor")).expect("create vendor dir");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let odb_count_before = count_odb_objects(&repo);
    let _id = SnapshotId::of_worktree(&repo, &worktree).expect("compute worktree id");
    let odb_count_after = count_odb_objects(&repo);

    assert_eq!(
        odb_count_before, odb_count_after,
        "computing a worktree ID must not write an object to the ODB"
    );
}

#[test]
fn test_staged_id_reads_the_index_not_the_worktree() {
    let (dir, repo) = common::init_repo();
    common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);
    stage_bytes(&repo, b"a.ts", MODE_REGULAR, b"staged");
    std::fs::write(dir.path().join("a.ts"), b"worktree").expect("write dirty worktree file");

    let index_id = SnapshotId::of_index(&IndexSnapshot::open(&repo).expect("open index snapshot"));
    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let worktree_id = SnapshotId::of_worktree(&repo, &worktree).expect("compute worktree id");
    assert_ne!(
        index_id, worktree_id,
        "staged content and worktree content differ, so the IDs must differ"
    );

    // Editing the worktree file further must not move the index ID: it
    // reads only the staged bytes.
    std::fs::write(dir.path().join("a.ts"), b"worktree again").expect("re-edit worktree file");
    let index_id_after =
        SnapshotId::of_index(&IndexSnapshot::open(&repo).expect("re-open index snapshot"));
    assert_eq!(
        index_id, index_id_after,
        "a worktree-only edit must not move the index ID"
    );
}

fn regex_matches_blake3_hex(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("blake3:") else {
        return false;
    };
    hex.len() == 32
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn write_worktree_files(root: &std::path::Path, entries: &[(Vec<u8>, i32, Vec<u8>)]) {
    for (path, _mode, content) in entries {
        let full_path = root.join(std::str::from_utf8(path).expect("test path is ASCII"));
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&full_path, content).expect("write worktree fixture file");
    }
}

fn chmod_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("stat file").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod file to executable");
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

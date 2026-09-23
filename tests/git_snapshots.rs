//! WS-1 acceptance: `git2`-backed `Commit`/`Index`/`Worktree` snapshots,
//! `RepoPath`'s D3 percent-escape rule, and merge-base resolution under
//! `NSD-G101`.

mod common;

use std::process::Command;

use git2::{IndexEntry, IndexTime, Oid, Repository, Signature};

use nsd::git::mergebase;
use nsd::git::snapshot::{CommitSnapshot, EntryKind, IndexSnapshot, WorktreeSnapshot};

const MODE_REGULAR: i32 = 0o100644;
const MODE_SYMLINK: i32 = 0o120000;
const MODE_SUBMODULE: i32 = 0o160000;

#[test]
fn staged_reads_index_not_worktree() {
    let (dir, repo) = common::init_repo();
    common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);
    stage_bytes(&repo, b"a.ts", MODE_REGULAR, b"A");
    std::fs::write(dir.path().join("a.ts"), b"B").expect("write worktree file");

    let commit_snapshot = CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot");
    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    assert_eq!(
        find_content(&commit_snapshot.entries, b"a.ts"),
        Some(b"base".to_vec())
    );
    assert_eq!(
        find_content(&index_snapshot.entries, b"a.ts"),
        Some(b"A".to_vec())
    );
    assert_eq!(
        find_content(&worktree_snapshot.entries, b"a.ts"),
        Some(b"B".to_vec())
    );
}

#[test]
fn staged_delete_absent_from_index_snapshot() {
    let (dir, repo) = common::init_repo();
    common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);
    std::fs::write(dir.path().join("a.ts"), b"base").expect("write worktree file");

    let mut index = repo.index().expect("open index");
    index
        .read_tree(
            &repo
                .head()
                .expect("HEAD")
                .peel_to_tree()
                .expect("HEAD tree"),
        )
        .expect("sync index to HEAD");
    index.write().expect("write index");
    index
        .remove_path(std::path::Path::new("a.ts"))
        .expect("git rm --cached a.ts");
    index.write().expect("write index after removal");

    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");
    assert!(
        !index_snapshot
            .entries
            .iter()
            .any(|entry| entry.path.as_bytes() == b"a.ts"),
        "a git rm --cached path must be absent from the Index snapshot"
    );
    assert!(
        dir.path().join("a.ts").exists(),
        "the file must still be present on disk"
    );
}

#[test]
fn commit_snapshot_lists_tree_entries_sorted() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[
            (b"b.ts".to_vec(), MODE_REGULAR, b"bbb".to_vec()),
            (b"a.ts".to_vec(), MODE_REGULAR, b"aaaa".to_vec()),
            (b"link.ts".to_vec(), MODE_SYMLINK, b"target.ts".to_vec()),
        ],
    );

    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot");
    let paths: Vec<&[u8]> = snapshot.entries.iter().map(|e| e.path.as_bytes()).collect();
    assert_eq!(
        paths,
        vec![
            b"a.ts".as_slice(),
            b"b.ts".as_slice(),
            b"link.ts".as_slice()
        ],
        "entries are sorted by raw path bytes (D10)"
    );

    let a_entry = snapshot
        .entries
        .iter()
        .find(|e| e.path.as_bytes() == b"a.ts")
        .expect("a.ts entry present");
    assert_eq!(a_entry.kind, EntryKind::Regular);
    assert_eq!(a_entry.size, 4);
    assert!(a_entry.oid.is_some(), "a blob entry carries its OID");
}

#[test]
fn unborn_repo_base_is_empty_tree() {
    let (_dir, repo) = common::init_repo();
    stage_bytes(&repo, b"a.ts", MODE_REGULAR, b"A");
    let odb_count_before = count_odb_objects(&repo);

    let base = CommitSnapshot::head_or_empty(&repo).expect("resolve the empty-tree base");
    assert!(
        base.entries.is_empty(),
        "an unborn HEAD resolves to the empty tree"
    );

    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");
    assert_eq!(index_snapshot.entries.len(), 1);
    assert_eq!(index_snapshot.entries[0].path.as_bytes(), b"a.ts");

    let odb_count_after = count_odb_objects(&repo);
    assert_eq!(
        odb_count_before, odb_count_after,
        "resolving the empty tree must not write an object to the ODB"
    );
}

#[test]
fn worktree_overlays_modified_deleted_and_untracked() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[
            (b"kept.ts".to_vec(), MODE_REGULAR, b"kept".to_vec()),
            (b"modified.ts".to_vec(), MODE_REGULAR, b"before".to_vec()),
            (b"deleted.ts".to_vec(), MODE_REGULAR, b"gone".to_vec()),
        ],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("kept.ts"), b"kept").expect("write kept.ts");
    std::fs::write(dir.path().join("modified.ts"), b"after").expect("write modified.ts");
    // deleted.ts is intentionally never written to disk (on-disk deletion).
    std::fs::write(dir.path().join(".gitignore"), b"untracked.ts\n").expect("write .gitignore");
    std::fs::write(dir.path().join("untracked.ts"), b"new").expect("write untracked.ts");

    let snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    assert_eq!(
        find_content(&snapshot.entries, b"kept.ts"),
        Some(b"kept".to_vec())
    );
    assert_eq!(
        find_content(&snapshot.entries, b"modified.ts"),
        Some(b"after".to_vec()),
        "worktree bytes come from disk (D5), overlaying the index"
    );
    assert!(
        !snapshot
            .entries
            .iter()
            .any(|entry| entry.path.as_bytes() == b"deleted.ts"),
        "an on-disk deletion is removed from the overlay"
    );
    assert_eq!(
        find_content(&snapshot.entries, b"untracked.ts"),
        Some(b"new".to_vec()),
        "an untracked file is added even when the candidate .gitignore matches it (D6)"
    );
}

#[test]
fn worktree_reports_nested_checkout_entry() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"kept.ts".to_vec(), MODE_REGULAR, b"kept".to_vec())],
    );
    sync_index_to_commit(&repo, commit_oid);
    std::fs::write(dir.path().join("kept.ts"), b"kept").expect("write kept.ts");

    let nested_dir = dir.path().join("vendor").join("nested-repo");
    std::fs::create_dir_all(&nested_dir).expect("create nested checkout directory");
    std::fs::write(nested_dir.join(".git"), b"gitdir: /elsewhere\n")
        .expect("write nested .git marker");
    std::fs::write(nested_dir.join("inner.ts"), b"inner")
        .expect("write file inside nested checkout");

    let snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let nested = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"vendor/nested-repo")
        .expect("the nested checkout is surfaced as one entry");
    assert_eq!(nested.kind, EntryKind::NestedCheckout);
    assert!(
        !snapshot
            .entries
            .iter()
            .any(|entry| entry.path.as_bytes().starts_with(b"vendor/nested-repo/")),
        "files inside a nested checkout are not listed"
    );
}

#[test]
fn entry_kinds_symlink_and_submodule() {
    let (_dir, repo) = common::init_repo();
    let gitlink_target = [0xABu8; 20];
    common::commit_entries(
        &repo,
        &[
            (b"link.ts".to_vec(), MODE_SYMLINK, b"target.ts".to_vec()),
            (
                b"vendor/lib".to_vec(),
                MODE_SUBMODULE,
                gitlink_target.to_vec(),
            ),
        ],
    );

    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot");
    let link = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"link.ts")
        .expect("symlink entry present");
    assert_eq!(link.kind, EntryKind::Symlink);
    assert!(
        link.content.is_none(),
        "a symlink is not readable as source bytes"
    );

    let submodule = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"vendor/lib")
        .expect("submodule entry present");
    assert_eq!(submodule.kind, EntryKind::Submodule);
    assert!(
        submodule.content.is_none(),
        "a submodule gitlink is not readable as source bytes"
    );
}

#[test]
fn conflicted_index_is_g101() {
    let (_dir, repo) = common::init_repo();
    let ancestor_oid =
        common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);
    let ours_oid =
        common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"ours".to_vec())]);
    let theirs_oid = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"theirs".to_vec())],
    );

    let ancestor_tree = repo
        .find_commit(ancestor_oid)
        .expect("ancestor commit")
        .tree()
        .expect("ancestor tree");
    let ours_tree = repo
        .find_commit(ours_oid)
        .expect("ours commit")
        .tree()
        .expect("ours tree");
    let theirs_tree = repo
        .find_commit(theirs_oid)
        .expect("theirs commit")
        .tree()
        .expect("theirs tree");

    let mut merged_index = repo
        .merge_trees(&ancestor_tree, &ours_tree, &theirs_tree, None)
        .expect("merge trees");
    assert!(
        merged_index.has_conflicts(),
        "fixture setup must actually produce a conflicted merge"
    );
    repo.set_index(&mut merged_index)
        .expect("install the conflicted index as the repo index");

    let err = IndexSnapshot::open(&repo).expect_err("a conflicted index snapshot must fail");
    assert_eq!(err.code(), "NSD-G101");
}

#[test]
fn non_utf8_path_round_trips_and_escapes() {
    let (_dir, repo) = common::init_repo();
    let mut invalid_path = b"src/a".to_vec();
    invalid_path.push(0xFF);
    invalid_path.extend_from_slice(b"%.ts");

    common::commit_entries(
        &repo,
        &[
            (invalid_path.clone(), MODE_REGULAR, b"content".to_vec()),
            (b"src/100%.ts".to_vec(), MODE_REGULAR, b"other".to_vec()),
        ],
    );

    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("open commit snapshot");
    let invalid_entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == invalid_path.as_slice())
        .expect("the raw non-UTF-8 path bytes round-trip");
    assert_eq!(invalid_entry.path.render(), "src/a%FF%25.ts");

    let valid_entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"src/100%.ts")
        .expect("a valid-UTF-8 path with % is preserved");
    assert_eq!(valid_entry.path.render(), "src/100%.ts");
}

#[test]
fn merge_base_full_clone_resolves() {
    let (_dir, repo) = common::init_repo();
    let base_oid =
        common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);
    let initial_branch = current_branch_name(&repo);
    let base_commit = repo.find_commit(base_oid).expect("base commit");
    repo.branch("feature", &base_commit, false)
        .expect("create feature branch");

    common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"on-main".to_vec())],
    );

    repo.set_head("refs/heads/feature")
        .expect("switch HEAD to feature");
    common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"on-feature".to_vec())],
    );
    repo.set_head(&format!("refs/heads/{initial_branch}"))
        .expect("switch HEAD back to the initial branch");

    let resolved =
        mergebase::merge_base(&repo, "feature").expect("merge base resolves in a full clone");
    assert_eq!(resolved, base_oid);
}

#[test]
fn merge_base_shallow_clone_is_g101() {
    let origin_dir = tempfile::TempDir::new().expect("temp origin dir");
    let origin = Repository::init(origin_dir.path()).expect("init origin repo");
    let root_oid = common::commit_entries(
        &origin,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"root".to_vec())],
    );
    let initial_branch = current_branch_name(&origin);
    let root_commit = origin.find_commit(root_oid).expect("root commit");
    origin
        .branch("feature", &root_commit, false)
        .expect("create feature branch");

    common::commit_entries(
        &origin,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"main2".to_vec())],
    );

    origin
        .set_head("refs/heads/feature")
        .expect("switch origin HEAD to feature");
    common::commit_entries(
        &origin,
        &[(b"a.ts".to_vec(), MODE_REGULAR, b"feature2".to_vec())],
    );
    origin
        .set_head(&format!("refs/heads/{initial_branch}"))
        .expect("switch origin HEAD back");

    let clone_dir = tempfile::TempDir::new().expect("temp clone dir");
    let status = Command::new("git")
        .args(["clone", "--depth", "1", "--no-single-branch"])
        .arg(format!("file://{}", origin_dir.path().display()))
        .arg(clone_dir.path())
        .status()
        .expect("spawn git clone");
    assert!(status.success(), "the shallow clone fixture must succeed");

    let clone_repo = Repository::open(clone_dir.path()).expect("open the shallow clone");
    assert!(
        clone_repo.is_shallow(),
        "the fixture must actually be a shallow clone"
    );

    let err = mergebase::merge_base(&clone_repo, "origin/feature")
        .expect_err("a merge base beyond the shallow boundary must fail");
    assert_eq!(err.code(), "NSD-G101");
    assert!(
        err.to_string().to_lowercase().contains("shallow"),
        "the message names the shallow clone: {err}"
    );
}

#[test]
fn merge_base_unresolvable_ref_and_unrelated_history_are_g101() {
    let (_unborn_dir, unborn_repo) = common::init_repo();
    let unborn_err = mergebase::merge_base(&unborn_repo, "HEAD")
        .expect_err("an unborn HEAD under --base must fail");
    assert_eq!(unborn_err.code(), "NSD-G101");

    let (_dir, repo) = common::init_repo();
    common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"base".to_vec())]);

    let unresolvable_err =
        mergebase::merge_base(&repo, "does-not-exist").expect_err("an unresolvable ref must fail");
    assert_eq!(unresolvable_err.code(), "NSD-G101");

    commit_orphan(&repo, b"b.ts", b"unrelated", "refs/heads/unrelated");
    let unrelated_err =
        mergebase::merge_base(&repo, "unrelated").expect_err("unrelated histories must fail");
    assert_eq!(unrelated_err.code(), "NSD-G101");
}

fn find_content(entries: &[nsd::git::snapshot::Entry], path: &[u8]) -> Option<Vec<u8>> {
    entries
        .iter()
        .find(|entry| entry.path.as_bytes() == path)
        .and_then(|entry| entry.content.clone())
}

fn current_branch_name(repo: &Repository) -> String {
    repo.head()
        .expect("HEAD after the first commit")
        .shorthand()
        .expect("HEAD has a shorthand branch name")
        .to_string()
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

/// Creates a single-file, parentless commit under `update_ref`, independent
/// of `HEAD`'s own history — the "unrelated histories" fixture.
fn commit_orphan(repo: &Repository, path: &[u8], content: &[u8], update_ref: &str) -> Oid {
    let mut builder = repo.treebuilder(None).expect("create treebuilder");
    let blob_oid = repo.blob(content).expect("write blob");
    builder
        .insert(path, blob_oid, MODE_REGULAR)
        .expect("insert tree entry");
    let tree_oid = builder.write().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let signature =
        Signature::now("nsd test fixture", "fixture@example.invalid").expect("build signature");
    repo.commit(
        Some(update_ref),
        &signature,
        &signature,
        "orphan commit",
        &tree,
        &[],
    )
    .expect("commit orphan")
}

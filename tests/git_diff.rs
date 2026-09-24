//! WS-2 acceptance: the central file-level Change listing (D8 renames at
//! `RENAME_THRESHOLD`, A6's two fixtures, commit/index/worktree diff modes,
//! D24's mempack seam, D10's sort) and the `map_lines` line-mapping
//! primitive (D9's forced text mode, D11's 1-based lines).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use git2::{IndexEntry, IndexTime, ObjectType, Oid, Repository, Signature};
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
        Change::Renamed {
            from,
            to,
            kind,
            similarity,
        } => {
            assert_eq!(from.as_bytes(), b"src/Old.java");
            assert_eq!(to.as_bytes(), b"src/New.java");
            assert_eq!(*kind, EntryKind::Regular);
            assert!(
                *similarity >= diff::RENAME_THRESHOLD,
                "expected similarity >= {}, got {similarity}",
                diff::RENAME_THRESHOLD
            );
            assert!(
                *similarity < 100,
                "one line of 40 differs, so this cannot be an exact match: got {similarity}"
            );
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
}

/// A user's `diff.renames`/`diff.renamelimit` config must never change
/// nsd's rename output (D8/D10: deterministic renames). `diff.renames` is
/// already ignored by the pre-fix code (`.renames(true)` is explicit, so
/// libgit2's own `normalize_find_opts` never even looks at the config: it
/// only consults `diff.renames` when `given->flags & GIT_DIFF_FIND_ALL ==
/// GIT_DIFF_FIND_BY_CONFIG`, `diff_tform.c:263-283`). `diff.renamelimit`
/// is genuinely read pre-fix, because `rename_limit` is otherwise left at
/// 0 ("unset"), `diff_tform.c:322-330`. Three unrelated deletions (not one,
/// as in `clear_rename_detected`) are needed to observe that: libgit2's own
/// rename-target loop only *breaks* once it has examined one more source
/// than `rename_limit` (`diff_tform.c:950-958`), so a `rename_limit` of 1
/// still fully examines 2 sources - only a 3rd source, alphabetically
/// after two unrelated ones, is left out, sending the true match past the
/// cap and turning the rename into a delete+add pre-fix.
#[test]
fn rename_detection_ignores_diff_renames_config() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"src/A.java".to_vec(), MODE_REGULAR, filler_lines(b'A', 40)),
            (b"src/B.java".to_vec(), MODE_REGULAR, filler_lines(b'B', 40)),
            (
                b"src/Old.java".to_vec(),
                MODE_REGULAR,
                numbered_lines(40, None),
            ),
        ],
    );
    sync_index_to_commit(&repo, base_oid);

    {
        let mut config = repo.config().expect("open repo config");
        config
            .set_str("diff.renames", "false")
            .expect("set diff.renames = false");
        config
            .set_str("diff.renamelimit", "1")
            .expect("set diff.renamelimit = 1");
    }

    let mut index = repo.index().expect("open index");
    for old_path in ["src/A.java", "src/B.java", "src/Old.java"] {
        index
            .remove_path(Path::new(old_path))
            .unwrap_or_else(|err| panic!("remove {old_path} from the index: {err}"));
    }
    index.write().expect("write index after removal");
    stage_bytes(
        &repo,
        b"src/New.java",
        MODE_REGULAR,
        &numbered_lines(40, Some((20, "line 20 EDITED"))),
    );

    let changes = diff::diff_commit_to_index(&repo, Some(base_oid)).expect("diff commit to index");

    let renamed = changes.iter().find(
        |c| matches!(c, Change::Renamed { from, to, .. } if from.as_bytes() == b"src/Old.java" && to.as_bytes() == b"src/New.java"),
    );
    assert!(
        renamed.is_some(),
        "expected src/Old.java -> src/New.java to still be detected as a rename regardless of \
         user diff.renames/diff.renamelimit config: {changes:?}"
    );
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

/// A renamed text file whose content is text-classified by libgit2 but not
/// valid UTF-8 must still report a similarity, not fail the whole diff.
#[test]
fn renamed_non_utf8_text_file_reports_similarity() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"a.js".to_vec(), MODE_REGULAR, non_utf8_lines(false))],
    );
    let candidate_oid = common::commit_entries(
        &repo,
        &[(b"b.js".to_vec(), MODE_REGULAR, non_utf8_lines(true))],
    );

    let changes = diff::diff_commit_to_commit(&repo, Some(base_oid), candidate_oid)
        .expect("diff commit to commit must not fail on non-UTF-8 renamed content");

    assert_eq!(changes.len(), 1, "expected exactly one change: {changes:?}");
    match &changes[0] {
        Change::Renamed {
            from,
            to,
            kind,
            similarity,
        } => {
            assert_eq!(from.as_bytes(), b"a.js");
            assert_eq!(to.as_bytes(), b"b.js");
            assert_eq!(*kind, EntryKind::Regular);
            assert!(
                *similarity >= diff::RENAME_THRESHOLD,
                "expected similarity >= {}, got {similarity}",
                diff::RENAME_THRESHOLD
            );
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
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

/// D32/D34: the same conflicted-index fixture as
/// `tests/git_snapshots.rs::conflicted_index_is_g101`, copied here so this
/// suite pins `diff_commit_to_index`'s own D4 behavior, not
/// `IndexSnapshot::open`'s (task.md Codex r1 test note).
#[test]
fn conflicted_index_diff_is_g101() {
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

    let err = diff::diff_commit_to_index(&repo, Some(ours_oid))
        .expect_err("diff_commit_to_index must reject a conflicted index");
    assert_eq!(err.code(), "NSD-G101");
    assert!(
        err.to_string().contains("unresolved merge conflicts"),
        "expected the D4 message, got: {err}"
    );
}

#[test]
fn exact_rename_reports_full_similarity() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"a.ts".to_vec(), MODE_REGULAR, filler_lines(b'Q', 40))],
    );
    let candidate_oid = common::commit_entries(
        &repo,
        &[(b"b.ts".to_vec(), MODE_REGULAR, filler_lines(b'Q', 40))],
    );

    let changes = diff::diff_commit_to_commit(&repo, Some(base_oid), candidate_oid)
        .expect("diff commit to commit");

    assert_eq!(changes.len(), 1, "expected exactly one change: {changes:?}");
    match &changes[0] {
        Change::Renamed {
            from,
            to,
            kind,
            similarity,
        } => {
            assert_eq!(from.as_bytes(), b"a.ts");
            assert_eq!(to.as_bytes(), b"b.ts");
            assert_eq!(*kind, EntryKind::Regular);
            assert_eq!(*similarity, 100, "an exact-content rename must score 100");
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
}

#[test]
fn commit_to_commit_reports_changes() {
    let (_dir, repo) = common::init_repo();
    let base_oid =
        common::commit_entries(&repo, &[(b"a.ts".to_vec(), MODE_REGULAR, b"a\n".to_vec())]);
    let candidate_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"a2\n".to_vec()),
            (b"b.ts".to_vec(), MODE_REGULAR, b"b\n".to_vec()),
        ],
    );

    let changes = diff::diff_commit_to_commit(&repo, Some(base_oid), candidate_oid)
        .expect("diff commit to commit");

    assert_eq!(
        changes,
        vec![
            Change::Modified {
                path: RepoPath::from_bytes(b"a.ts".to_vec()),
                kind: EntryKind::Regular,
            },
            Change::Added {
                path: RepoPath::from_bytes(b"b.ts".to_vec()),
                kind: EntryKind::Regular,
            },
        ]
    );
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
fn worktree_diff_keeps_symlink_and_gitlink_modes() {
    // D34: the gitlink lives at `third_party/lib`, not `vendor/lib` — D28
    // masks every `vendor/**` path as a built-in exclusion, which would make
    // this test's "unchanged tracked gitlink must not appear" half pass
    // vacuously (masked regardless of any real change) instead of actually
    // exercising diff_commit_to_worktree's own unchanged-gitlink handling.
    let (dir, repo) = common::init_repo();
    let gitlink = [0xAAu8; 20];
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"link".to_vec(), MODE_SYMLINK, b"target.ts".to_vec()),
            (
                b"third_party/lib".to_vec(),
                MODE_SUBMODULE,
                gitlink.to_vec(),
            ),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    std::os::unix::fs::symlink("target.ts", dir.path().join("link"))
        .expect("create a real symlink matching the committed target");
    std::fs::create_dir_all(dir.path().join("third_party/lib"))
        .expect("create the tracked submodule's directory");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");
    assert_eq!(
        changes,
        Vec::new(),
        "an unchanged tracked symlink/gitlink must not appear: {changes:?}"
    );

    std::fs::remove_file(dir.path().join("link")).expect("remove the old symlink");
    std::os::unix::fs::symlink("other.ts", dir.path().join("link")).expect("re-point the symlink");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");
    assert_eq!(
        changes,
        vec![Change::Modified {
            path: RepoPath::from_bytes(b"link".to_vec()),
            kind: EntryKind::Symlink,
        }]
    );
}

/// D28 (user decision (b)): every built-in-excluded path is masked out of
/// `diff_commit_to_worktree` before Phase 1 reads or hashes anything, on
/// both sides and for every entry kind and extension. `target/locked.ts`'s
/// mode `000` proves the point operationally: if masking ran after a read
/// attempt (or not at all), the whole diff would fail closed on that one
/// unreadable path instead of returning cleanly.
#[test]
#[cfg(unix)]
fn worktree_diff_masks_builtin_excluded_paths() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, repo) = common::init_repo();
    let gone_content = numbered_lines(40, None);
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"src/gone.ts".to_vec(), MODE_REGULAR, gone_content.clone()),
            (
                b"vendor/kept.js".to_vec(),
                MODE_REGULAR,
                b"vendor original\n".to_vec(),
            ),
            (
                b"node_modules/old.js".to_vec(),
                MODE_REGULAR,
                b"old\n".to_vec(),
            ),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    // `common::commit_entries` never writes to disk (D23): src/gone.ts and
    // node_modules/old.js are already on-disk deletions.
    std::fs::create_dir_all(dir.path().join("vendor")).expect("create vendor dir");
    std::fs::write(dir.path().join("vendor/kept.js"), b"vendor edited\n")
        .expect("edit vendor/kept.js on disk");

    std::fs::create_dir_all(dir.path().join("target/debug")).expect("create target/debug dir");
    std::fs::write(dir.path().join("target/debug/copy.ts"), &gone_content)
        .expect("write a byte-identical copy of src/gone.ts under target/");
    std::fs::write(dir.path().join("target/app.rlib"), b"rlib\n")
        .expect("write target/app.rlib (no supported extension)");
    let over_ceiling = vec![b'z'; SOURCE_CEILING_BYTES as usize + 1];
    std::fs::write(dir.path().join("target/big.ts"), &over_ceiling)
        .expect("write an over-ceiling file under target/");
    std::fs::write(dir.path().join("target/locked.ts"), b"locked\n")
        .expect("write target/locked.ts before locking it down");
    let mut perms = std::fs::metadata(dir.path().join("target/locked.ts"))
        .expect("stat target/locked.ts")
        .permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(dir.path().join("target/locked.ts"), perms)
        .expect("chmod target/locked.ts to 000");
    assert!(
        std::fs::File::open(dir.path().join("target/locked.ts")).is_err(),
        "precondition: target/locked.ts must be unreadable"
    );

    std::fs::create_dir_all(dir.path().join("dist")).expect("create dist dir");
    std::fs::write(dir.path().join("dist/y.js"), b"dist\n").expect("write dist/y.js");

    write_nested_checkout(&dir, "node_modules/pkg");

    // WS-4 r2 row 4: a `Special` entry (never read) at a built-in-excluded
    // path must also be masked — proves the built-in check runs ahead of
    // `entry.kind`, not only for the kinds already covered above.
    let status = Command::new("/usr/bin/mkfifo")
        .arg(dir.path().join("target/pipe"))
        .status()
        .expect("spawn mkfifo for target/pipe");
    assert!(status.success(), "mkfifo target/pipe must succeed");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = with_timeout(
        "diff_commit_to_worktree over built-in-excluded paths",
        move || diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree),
    )
    .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![Change::Deleted {
            path: RepoPath::from_bytes(b"src/gone.ts".to_vec()),
            kind: EntryKind::Regular,
        }],
        "every built-in-excluded path must be masked out, on both sides and \
         regardless of kind or extension: {changes:?}"
    );
}

/// D28: a base blob at a built-in-excluded path can never be a rename
/// source, because the base side is masked into the in-memory index too —
/// otherwise every deleted `vendor/**` file would look like a rename source
/// for an unrelated, non-excluded add with the same content.
#[test]
fn worktree_diff_builtin_path_is_never_a_rename_source() {
    let (dir, repo) = common::init_repo();
    let content = numbered_lines(40, None);
    let base_oid = common::commit_entries(
        &repo,
        &[(b"vendor/lib.ts".to_vec(), MODE_REGULAR, content.clone())],
    );
    sync_index_to_commit(&repo, base_oid);
    // `common::commit_entries` never writes to disk (D23): vendor/lib.ts is
    // already an on-disk deletion; write only the unrelated, byte-identical
    // add.
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::write(dir.path().join("src/lib.ts"), &content)
        .expect("write a byte-identical unrelated add");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![Change::Added {
            path: RepoPath::from_bytes(b"src/lib.ts".to_vec()),
            kind: EntryKind::Regular,
        }],
        "a masked base blob must never pair with a non-excluded add as a rename: {changes:?}"
    );
}

/// WS-4 r2 row 1: the built-in predicate must also match a directory-kind
/// worktree entry (`NestedCheckout`/`Submodule`) whose own path *is* the
/// built-in directory, not only a leaf nested beneath one. Pre-fix, every
/// check passed `is_dir = false` regardless of the entry's actual kind, so a
/// directory-anchored glob such as `target/` or `vendor/` never matched the
/// directory entry itself.
#[test]
#[cfg(unix)]
fn worktree_diff_masks_directory_kind_entry_at_builtin_dir() {
    let (dir, repo) = common::init_repo();
    let gitlink = [0xBBu8; 20];
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"a.ts".to_vec(), MODE_REGULAR, b"unchanged\n".to_vec()),
            (b"vendor".to_vec(), MODE_SUBMODULE, gitlink.to_vec()),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::write(dir.path().join("a.ts"), b"unchanged\n").expect("write a.ts unchanged");
    // No `vendor` directory is ever created on disk: the base gitlink at
    // `vendor` must be masked from the base side alone.
    write_nested_checkout(&dir, "target");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes,
        Vec::new(),
        "a directory-kind worktree entry and a base gitlink at a built-in \
         directory's own path must both be masked: {changes:?}"
    );
}

/// WS-4 r2 row 2: a worktree *leaf* (Regular/Symlink) whose own path equals
/// a masked built-in directory's name must not un-mask that directory's
/// base entries. Pre-fix, inserting such a leaf into the libgit2 in-memory
/// index removed every already-masked `vendor/**` base row
/// (`index_insert(replace=1)` -> `has_file_name`, libgit2-sys-0.18.8
/// `index.c:1142-1147`, `:1706`), so those rows came back as `Deleted` and
/// became rename sources for the leaf.
#[test]
fn worktree_diff_leaf_replacing_masked_builtin_dir_keeps_mask() {
    let (dir, repo) = common::init_repo();
    let content = numbered_lines(40, None);
    let base_oid = common::commit_entries(
        &repo,
        &[(b"vendor/lib.ts".to_vec(), MODE_REGULAR, content.clone())],
    );
    sync_index_to_commit(&repo, base_oid);
    // No `vendor/` directory on disk at all: `vendor` itself is a regular
    // file whose bytes exactly match the masked base blob.
    std::fs::write(dir.path().join("vendor"), &content).expect("write leaf vendor file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![Change::Added {
            path: RepoPath::from_bytes(b"vendor".to_vec()),
            kind: EntryKind::Regular,
        }],
        "a leaf replacing a masked built-in directory must never un-mask it \
         or become a rename target: {changes:?}"
    );
}

/// Symlink variant of `worktree_diff_leaf_replacing_masked_builtin_dir_keeps_mask`.
#[test]
fn worktree_diff_symlink_leaf_replacing_masked_builtin_dir_keeps_mask() {
    let (dir, repo) = common::init_repo();
    let content = numbered_lines(40, None);
    let base_oid =
        common::commit_entries(&repo, &[(b"vendor/lib.ts".to_vec(), MODE_REGULAR, content)]);
    sync_index_to_commit(&repo, base_oid);
    // No `vendor/` directory on disk at all: `vendor` itself is a symlink.
    std::os::unix::fs::symlink("elsewhere", dir.path().join("vendor"))
        .expect("create symlink leaf vendor");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![Change::Added {
            path: RepoPath::from_bytes(b"vendor".to_vec()),
            kind: EntryKind::Symlink,
        }],
        "a symlink leaf replacing a masked built-in directory must never \
         un-mask it or become a rename target: {changes:?}"
    );
}

/// WS-4 r2 row 3: `mask_builtin_base_entries`'s base-tree walk must tolerate
/// a non-UTF-8 directory name (D3/D18 supports non-UTF-8 paths generally).
/// git2's `Tree::walk` passes its `root` argument as `&str` and aborts the
/// whole walk when a directory name is not valid UTF-8.
#[test]
fn worktree_diff_tolerates_non_utf8_base_directory() {
    let (_dir, repo) = common::init_repo();
    let mut dir_name = b"caf".to_vec();
    dir_name.push(0xE9);
    let mut path = dir_name;
    path.push(b'/');
    path.extend_from_slice(b"a.ts");
    let base_oid = common::commit_entries(
        &repo,
        &[(path.clone(), MODE_REGULAR, b"content\n".to_vec())],
    );
    sync_index_to_commit(&repo, base_oid);
    // The worktree is empty: this base file is an on-disk deletion, and no
    // non-UTF-8 name is ever created on disk (D23).

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree must tolerate a non-UTF-8 base directory name");

    assert_eq!(
        changes,
        vec![Change::Deleted {
            path: RepoPath::from_bytes(path),
            kind: EntryKind::Regular,
        }],
        "a non-UTF-8 base directory name must not abort the built-in-exclusion walk: {changes:?}"
    );
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
    let base_oid =
        common::commit_entries(&repo, &[(b"src/x.ts".to_vec(), MODE_REGULAR, Vec::new())]);
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
    let base_oid =
        common::commit_entries(&repo, &[(b"empty.ts".to_vec(), MODE_REGULAR, Vec::new())]);
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

    assert_eq!(
        changes.len(),
        2,
        "expected a delete and an unrelated add, not a rename: {changes:?}"
    );
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Deleted { path, .. } if path.as_bytes() == b"empty.ts")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"big.js")));
}

#[test]
fn worktree_over_ceiling_add_does_not_disable_renames_elsewhere() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"empty.ts".to_vec(), MODE_REGULAR, Vec::new()),
            (
                b"src/Old.java".to_vec(),
                MODE_REGULAR,
                numbered_lines(40, None),
            ),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    // `commit_entries` never writes to disk (D23): `empty.ts` and
    // `src/Old.java` are already on-disk deletions; write only the renamed
    // replacement and an unrelated untracked over-ceiling file.
    std::fs::create_dir_all(dir.path().join("src")).expect("create src/ directory");
    std::fs::write(
        dir.path().join("src/New.java"),
        numbered_lines(40, Some((20, "line 20 EDITED"))),
    )
    .expect("write the renamed replacement");
    let over_ceiling = vec![b'x'; SOURCE_CEILING_BYTES as usize + 1];
    std::fs::write(dir.path().join("big.js"), &over_ceiling)
        .expect("write an unrelated untracked over-ceiling file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes.len(),
        3,
        "an unrelated over-ceiling add must not disable rename detection for src/Old.java: {changes:?}"
    );
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Added { path, .. } if path.as_bytes() == b"big.js")));
    assert!(changes
        .iter()
        .any(|c| matches!(c, Change::Deleted { path, .. } if path.as_bytes() == b"empty.ts")));
    let renamed = changes.iter().find(|c| matches!(c, Change::Renamed { .. }));
    match renamed {
        Some(Change::Renamed {
            from,
            to,
            kind,
            similarity,
        }) => {
            assert_eq!(from.as_bytes(), b"src/Old.java");
            assert_eq!(to.as_bytes(), b"src/New.java");
            assert_eq!(*kind, EntryKind::Regular);
            assert!(
                *similarity >= diff::RENAME_THRESHOLD,
                "expected similarity >= {}, got {similarity}",
                diff::RENAME_THRESHOLD
            );
        }
        other => panic!("expected a Renamed change among {changes:?}, got {other:?}"),
    }
}

#[test]
fn worktree_over_ceiling_exact_rename_is_renamed() {
    let (dir, repo) = common::init_repo();
    let over_ceiling = vec![b'd'; SOURCE_CEILING_BYTES as usize + 1];
    let base_oid = common::commit_entries(
        &repo,
        &[(b"big.js".to_vec(), MODE_REGULAR, over_ceiling.clone())],
    );
    sync_index_to_commit(&repo, base_oid);
    // `commit_entries` never writes to disk (D23): `big.js` is already an
    // on-disk deletion; write only the renamed replacement, byte-identical.
    std::fs::write(dir.path().join("big2.js"), &over_ceiling)
        .expect("write the exact-OID renamed replacement");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(changes.len(), 1, "expected exactly one change: {changes:?}");
    match &changes[0] {
        Change::Renamed {
            from,
            to,
            kind,
            similarity,
        } => {
            assert_eq!(from.as_bytes(), b"big.js");
            assert_eq!(to.as_bytes(), b"big2.js");
            assert_eq!(*kind, EntryKind::Regular);
            assert_eq!(*similarity, 100, "identical bytes are an exact match");
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
}

/// An over-ceiling worktree entry whose base side is a Typechange (not an
/// Added) must stay one `Typechange` delta, never split into a same-path
/// `Deleted` + `Added` pair by the over-ceiling reconciliation.
#[test]
fn worktree_over_ceiling_typechange_stays_typechange() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[
            (b"gone.ts".to_vec(), MODE_REGULAR, b"x\n".to_vec()),
            (b"link".to_vec(), MODE_SYMLINK, b"target.ts".to_vec()),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    // `commit_entries` never writes to disk (D23): `gone.ts` is already an
    // on-disk deletion. `link` becomes an over-ceiling regular file on disk,
    // type-changed from its base symlink.
    let over_ceiling = vec![b'e'; SOURCE_CEILING_BYTES as usize + 1];
    std::fs::write(dir.path().join("link"), &over_ceiling)
        .expect("write link as an over-ceiling regular file");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![
            Change::Deleted {
                path: RepoPath::from_bytes(b"gone.ts".to_vec()),
                kind: EntryKind::Regular,
            },
            Change::Typechange {
                path: RepoPath::from_bytes(b"link".to_vec()),
                old_kind: EntryKind::Symlink,
                new_kind: EntryKind::Regular,
            },
        ],
        "an over-ceiling typechange must stay one Typechange, not split into Deleted+Added: {changes:?}"
    );
}

/// D33: runs `f` on a detached, `'static` thread and waits at most 5s for
/// its result. A hang inside `f` (the defect this guards against, e.g. an
/// `open(2)` on a FIFO with no writer) leaves the thread running forever,
/// but the test process still exits: nothing here ever joins it.
#[cfg(unix)]
fn with_timeout<T: Send + 'static>(label: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(value) => value,
        Err(_) => panic!("{label} did not complete within the 5s guard; it is likely blocked"),
    }
}

#[test]
#[cfg(unix)]
fn worktree_special_files_reported_by_kind_never_opened() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[
            (
                b"src/tracked.ts".to_vec(),
                MODE_REGULAR,
                b"before\n".to_vec(),
            ),
            (b"src/gone.ts".to_vec(), MODE_REGULAR, b"bye\n".to_vec()),
        ],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    // src/gone.ts is intentionally never written to disk (on-disk deletion,
    // whose old side is a regular blob, fires the Phase-2 rename gate).
    let status = Command::new("/usr/bin/mkfifo")
        .arg(dir.path().join("src/tracked.ts"))
        .status()
        .expect("spawn mkfifo for the replaced src/tracked.ts");
    assert!(status.success(), "mkfifo src/tracked.ts must succeed");
    let status = Command::new("/usr/bin/mkfifo")
        .arg(dir.path().join("src/new.ts"))
        .status()
        .expect("spawn mkfifo for the untracked src/new.ts");
    assert!(status.success(), "mkfifo src/new.ts must succeed");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = with_timeout("diff_commit_to_worktree over Special entries", move || {
        diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
    })
    .expect("diff commit to worktree");

    assert_eq!(
        changes,
        vec![
            Change::Deleted {
                path: RepoPath::from_bytes(b"src/gone.ts".to_vec()),
                kind: EntryKind::Regular,
            },
            Change::Added {
                path: RepoPath::from_bytes(b"src/new.ts".to_vec()),
                kind: EntryKind::Special,
            },
            Change::Typechange {
                path: RepoPath::from_bytes(b"src/tracked.ts".to_vec()),
                old_kind: EntryKind::Regular,
                new_kind: EntryKind::Special,
            },
        ],
        "a Special entry is reported by kind (Added when new, Typechange over a base blob), \
         never read, and never a rename source/target: {changes:?}"
    );
}

/// Row 1: `WorktreeSnapshot::read` returns `Ok(None)` for a Regular entry
/// (classified at enumeration time) swapped to a FIFO before the read
/// happens (D25's re-check). `worktree_blob_oid`'s `None` arm must not treat
/// that the same as an over-ceiling file and fall through to
/// `Oid::hash_file`, which would `open(2)` the FIFO and block.
#[test]
#[cfg(unix)]
fn worktree_diff_entry_swapped_to_fifo_after_snapshot_fails_promptly() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"src/a.ts".to_vec(), MODE_REGULAR, b"before\n".to_vec())],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::write(dir.path().join("src/a.ts"), b"before\n").expect("write src/a.ts");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let entry = worktree
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"src/a.ts")
        .expect("src/a.ts entry present");
    assert_eq!(
        entry.kind,
        EntryKind::Regular,
        "classified before the swap, at enumeration time"
    );

    std::fs::remove_file(dir.path().join("src/a.ts")).expect("remove src/a.ts before the swap");
    let status = Command::new("/usr/bin/mkfifo")
        .arg(dir.path().join("src/a.ts"))
        .status()
        .expect("spawn mkfifo for the swapped src/a.ts");
    assert!(status.success(), "mkfifo src/a.ts must succeed");

    let result = with_timeout(
        "diff_commit_to_worktree over a Regular entry swapped to a FIFO after the snapshot",
        move || diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree),
    );

    let err =
        result.expect_err("a Regular entry swapped to a FIFO after the snapshot must fail closed");
    assert_eq!(err.code(), nsd::git::CODE_SNAPSHOT_UNAVAILABLE);
}

/// Row 3 (D27): a base tree entry at `path` can itself be a directory (mode
/// `040000`) rather than a file, when the worktree's Special file replaces a
/// whole tracked subtree. `old_side_entry` must not hand that tree mode to
/// `entry_kind_from_raw_mode` (which rejects it, failing the whole diff with
/// NSD-G101); it has no old-side *file* to mask against, so the Special path
/// is reported as newly Added, with the base's own file entries listed
/// separately as Deleted.
#[test]
#[cfg(unix)]
fn worktree_special_file_replacing_base_directory_is_added() {
    // WS-4 r1 upkeep: this fixture's directory is named `src/sub`, not
    // `src/gen` — the latter collides with D19/D28's built-in
    // `**/gen/**` GENERATED_GLOBS exclusion, which would mask `a.ts`'s base
    // entry into the index and turn the `Deleted` half of this assertion
    // vacuous instead of exercising row 3's own directory-replacement logic.
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(
            b"src/sub/a.ts".to_vec(),
            MODE_REGULAR,
            b"generated\n".to_vec(),
        )],
    );
    sync_index_to_commit(&repo, base_oid);
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    // `src/sub` is intentionally never created as a directory: the base's
    // own `src/sub/a.ts` therefore has no on-disk file at all (an on-disk
    // deletion), and `src/sub` itself is a FIFO where the base tree instead
    // has a directory entry.
    let status = Command::new("/usr/bin/mkfifo")
        .arg(dir.path().join("src/sub"))
        .status()
        .expect("spawn mkfifo for src/sub");
    assert!(status.success(), "mkfifo src/sub must succeed");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let changes = with_timeout(
        "diff_commit_to_worktree with a Special entry replacing a base directory",
        move || diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree),
    )
    .expect("diff commit to worktree");

    // Observed order: the final sort in `diff_commit_to_worktree` sorts by
    // raw path bytes, and "src/sub" is a byte-prefix of "src/sub/a.ts", so
    // the shorter path sorts first.
    assert_eq!(
        changes,
        vec![
            Change::Added {
                path: RepoPath::from_bytes(b"src/sub".to_vec()),
                kind: EntryKind::Special,
            },
            Change::Deleted {
                path: RepoPath::from_bytes(b"src/sub/a.ts".to_vec()),
                kind: EntryKind::Regular,
            },
        ],
        "a base directory replaced on disk by a Special file must be Added{{Special}}, with \
         the base's own file entries listed separately as Deleted, never NSD-G101: {changes:?}"
    );
}

#[test]
fn worktree_content_rename_detected() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(
            b"src/Old.java".to_vec(),
            MODE_REGULAR,
            numbered_lines(40, None),
        )],
    );
    sync_index_to_commit(&repo, base_oid);
    // `commit_entries` never writes to disk (D23): `src/Old.java` is already
    // an on-disk deletion; write only the renamed replacement.
    std::fs::create_dir_all(dir.path().join("src")).expect("create src/ directory");
    std::fs::write(
        dir.path().join("src/New.java"),
        numbered_lines(40, Some((20, "line 20 EDITED"))),
    )
    .expect("write the renamed replacement");

    let worktree = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");
    let odb_objects_before = count_odb_objects(&repo);
    let changes = diff::diff_commit_to_worktree(&repo, Some(base_oid), &worktree)
        .expect("diff commit to worktree");
    let odb_objects_after = count_odb_objects(&repo);

    assert_eq!(
        odb_objects_before, odb_objects_after,
        "a content rename must not write to the ODB (D24)"
    );
    assert_eq!(changes.len(), 1, "expected exactly one change: {changes:?}");
    match &changes[0] {
        Change::Renamed {
            from,
            to,
            kind,
            similarity,
        } => {
            assert_eq!(from.as_bytes(), b"src/Old.java");
            assert_eq!(to.as_bytes(), b"src/New.java");
            assert_eq!(*kind, EntryKind::Regular);
            assert!(
                *similarity >= diff::RENAME_THRESHOLD,
                "expected similarity >= {}, got {similarity}",
                diff::RENAME_THRESHOLD
            );
        }
        other => panic!("expected a Renamed change, got {other:?}"),
    }
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
fn nonstandard_tree_mode_is_normalized_not_panicking() {
    let (_dir, repo) = common::init_repo();
    let blob_oid = repo.blob(b"a\n").expect("write a.ts blob");

    let mut tree_bytes = Vec::new();
    tree_bytes.extend_from_slice(b"100600 a.ts\0");
    tree_bytes.extend_from_slice(blob_oid.as_bytes());
    let tree_oid = repo
        .odb()
        .expect("open odb")
        .write(ObjectType::Tree, &tree_bytes)
        .expect("write a tree with a nonstandard raw mode");
    let tree = repo
        .find_tree(tree_oid)
        .expect("read back the nonstandard-mode tree");

    let signature =
        Signature::now("nsd test fixture", "fixture@example.invalid").expect("build a signature");
    let candidate_oid = repo
        .commit(None, &signature, &signature, "nonstandard mode", &tree, &[])
        .expect("commit the nonstandard-mode tree");

    let changes = diff::diff_commit_to_commit(&repo, None, candidate_oid)
        .expect("diff commit to commit must not panic on a nonstandard raw tree mode");

    assert_eq!(
        changes,
        vec![Change::Added {
            path: RepoPath::from_bytes(b"a.ts".to_vec()),
            kind: EntryKind::Regular,
        }]
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
fn line_map_maps_lines_far_from_change() {
    let mut base = String::new();
    for i in 1..=20 {
        base.push_str(&format!("l{i}\n"));
    }
    let mut candidate = base.clone();
    candidate.push_str("tail\n");

    let map = diff::map_lines(base.as_bytes(), candidate.as_bytes()).expect("map lines");

    assert_eq!(
        map.base_to_candidate,
        (1..=20).map(|i| (i, i)).collect::<BTreeMap<_, _>>()
    );
    assert_eq!(map.added_candidate_lines, BTreeSet::from([21]));
    assert!(map.deleted_base_lines.is_empty());
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

/// Forty lines, one of which (`line 1`) holds a raw `0xE9` byte followed by
/// a non-continuation byte, so the buffer as a whole is not valid UTF-8, yet
/// stays small enough for libgit2's `git_str_is_binary` to still classify
/// it as text.
fn non_utf8_lines(edit_line0: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    if edit_line0 {
        buf.extend_from_slice(b"line 0 EDITED\n");
    } else {
        buf.extend_from_slice(b"line 0\n");
    }
    buf.extend_from_slice(b"var s = '\xe9';\n");
    for i in 2..40 {
        buf.extend_from_slice(format!("line {i}\n").as_bytes());
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

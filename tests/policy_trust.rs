//! Base-policy trust resolution (M2-2): the effective `Config` always
//! comes from a trusted `--config` file or the **base** snapshot, never
//! from the candidate under inspection (`nsd-plan-implementation.md:76`).

mod common;

use git2::{IndexEntry, IndexTime, ObjectType, Oid, Repository};

use nsd::config::{Config, Severity, CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG};
use nsd::git::snapshot::{CommitSnapshot, IndexSnapshot, WorktreeSnapshot, SOURCE_CEILING_BYTES};
use nsd::policy::{self, Candidate, ConfigSource, Diagnostic};

/// This suite's own fixture constant (D22): a regular file's Git mode,
/// used only here.
const MODE_REGULAR: i32 = 0o100644;
/// This suite's own fixture constant (D22, copied from `tests/git_diff.rs`
/// and siblings): a symlink's Git mode, used only here.
const MODE_SYMLINK: i32 = 0o120000;

/// Overwrites the Git index to exactly mirror `commit_oid`'s tree (D22,
/// copied from `tests/git_snapshots.rs`'s own helper of the same name):
/// `common::commit_entries` never touches the index itself.
fn sync_index_to_commit(repo: &Repository, commit_oid: Oid) {
    let commit = repo.find_commit(commit_oid).expect("find commit");
    let tree = commit.tree().expect("commit tree");
    let mut index = repo.index().expect("open index");
    index.read_tree(&tree).expect("read tree into index");
    index.write().expect("write index");
}

/// Stages `content` at `path` directly into the Git index (D22, copied
/// from `tests/git_snapshots.rs`'s own helper of the same name), without
/// writing a blob to the ODB or a file to disk.
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

/// Valid `version: 1` YAML padded past `SOURCE_CEILING_BYTES` with a
/// trailing comment line (triage-ws2-r1.md row 3): a ceiling guard that
/// were deleted would still reject an all-`#` fixture for its missing
/// `version` field, so the padding must stay parseable to actually
/// exercise the byte-length check.
fn oversized_valid_config() -> Vec<u8> {
    let mut bytes = b"version: 1\n".to_vec();
    bytes.resize((SOURCE_CEILING_BYTES + 1) as usize, b'#');
    bytes
}

#[test]
fn test_candidate_config_cannot_weaken_its_own_check() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\ninclude: [src/**]\nexclude: [vendor/**]\nmeasurement:\n  \
              min_clone_lines: 10\npolicy:\n  NSD-E101: deny\n"
                .to_vec(),
        )],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let base_config = nsd::config::load_from_commit(&repo, &base).expect("base config is valid");

    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\ninclude: [tests/**]\nexclude: [dist/**]\nmeasurement:\n  \
              min_clone_lines: 3\npolicy:\n  NSD-E101: off\n"
                .to_vec(),
        )],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("a valid base and a shape-valid candidate resolve");

    assert_eq!(
        resolution.config, base_config,
        "the whole effective Config must equal the base's, not a blend with the candidate"
    );
    assert_eq!(resolution.config.policy.nsd_e101, Severity::Deny);
    assert_eq!(resolution.source, ConfigSource::Base);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }]
    );
}

#[test]
fn test_first_config_uses_builtin_defaults() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"README.md".to_vec(), MODE_REGULAR, b"hi".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(
        &repo,
        &[
            (b"README.md".to_vec(), MODE_REGULAR, b"hi".to_vec()),
            (b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec()),
        ],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("no config anywhere plus a valid new one resolves");

    assert_eq!(resolution.config, Config::default());
    assert_eq!(resolution.source, ConfigSource::BuiltInDefaults);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }]
    );
}

#[test]
fn test_removed_candidate_config_still_uses_base() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\nmeasurement:\n  min_clone_lines: 20\n".to_vec(),
        )],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let base_config = nsd::config::load_from_commit(&repo, &base).expect("base config is valid");

    common::commit_entries(
        &repo,
        &[(b"README.md".to_vec(), MODE_REGULAR, b"hi".to_vec())],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("a removed candidate config still resolves through the base");

    assert_eq!(resolution.config, base_config);
    assert_eq!(resolution.source, ConfigSource::Base);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }]
    );
}

#[test]
fn test_trusted_config_replaces_repository_policy() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\nmeasurement:\n  min_clone_lines: 20\n".to_vec(),
        )],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(
        &trusted_path,
        b"version: 1\nmeasurement:\n  min_clone_lines: 99\n",
    )
    .expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a valid trusted config resolves");

    assert_eq!(resolution.config.measurement.min_clone_lines, 99);
    assert_eq!(resolution.source, ConfigSource::Trusted);
}

#[test]
fn test_trusted_config_overrides_an_invalid_base() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 2\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a trusted config overrides an invalid base");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(resolution.config, Config::default());
}

#[test]
fn test_trusted_config_overrides_an_unreadable_base() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a trusted config overrides an unreadable base");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(resolution.config, Config::default());
    assert!(
        resolution.diagnostics.is_empty(),
        "an unreadable base skips the candidate diff entirely"
    );
}

#[test]
fn test_invalid_or_missing_trusted_config_is_c102() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");

    let missing_path = trusted_dir.path().join("missing.yml");
    let err = policy::resolve(
        &repo,
        Some(missing_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect_err("a missing trusted config is C102");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);

    let invalid_path = trusted_dir.path().join("invalid.yml");
    std::fs::write(&invalid_path, b"version: 2\n").expect("write invalid trusted config");
    let err = policy::resolve(
        &repo,
        Some(invalid_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect_err("an invalid trusted config is C102");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);

    let oversized_path = trusted_dir.path().join("oversized.yml");
    std::fs::write(&oversized_path, oversized_valid_config())
        .expect("write oversized trusted config");
    let err = policy::resolve(
        &repo,
        Some(oversized_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect_err("an over-ceiling trusted config is C102");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
}

#[test]
fn test_invalid_base_config_is_c102() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 2\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let err = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect_err("an invalid base config with no trusted override is C102");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
}

#[test]
fn test_invalid_candidate_config_is_reported_but_base_still_applies() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let base_config = nsd::config::load_from_commit(&repo, &base).expect("base config is valid");

    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 2\n".to_vec())],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("an invalid candidate must not fail the whole resolution");

    assert_eq!(resolution.config, base_config);
    assert_eq!(resolution.source, ConfigSource::Base);
    assert_eq!(
        resolution.diagnostics,
        vec![
            Diagnostic {
                code: CODE_CONFIG_CHANGED
            },
            Diagnostic {
                code: CODE_INVALID_CONFIG
            },
        ]
    );
}

#[test]
fn test_oversized_candidate_config_is_reported_but_base_still_applies() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let base_config = nsd::config::load_from_commit(&repo, &base).expect("base config is valid");

    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("an oversized candidate must not fail the whole resolution");

    assert_eq!(resolution.config, base_config);
    assert_eq!(resolution.source, ConfigSource::Base);
    assert_eq!(
        resolution.diagnostics,
        vec![
            Diagnostic {
                code: CODE_CONFIG_CHANGED
            },
            Diagnostic {
                code: CODE_INVALID_CONFIG
            },
        ]
    );
}

#[test]
fn test_unchanged_candidate_config_reports_nothing() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let candidate = base.clone();

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate))
        .expect("an identical candidate resolves cleanly");

    assert!(resolution.diagnostics.is_empty());
}

#[test]
fn test_c101_is_informational_not_an_error() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\nmeasurement:\n  min_clone_lines: 5\n".to_vec(),
        )],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Commit(&candidate)).expect(
        "a changed but valid candidate is Ok, since C101 is informational (M5-3 maps exit codes)",
    );

    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }]
    );
}

#[test]
fn test_staged_candidate_reads_the_index_not_the_worktree() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    sync_index_to_commit(&repo, commit_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    // The worktree copy diverges from both the base and the (unmodified)
    // index.
    std::fs::write(
        dir.path().join("nsd.yml"),
        b"version: 1\nmeasurement:\n  min_clone_lines: 5\n",
    )
    .expect("write a diverging worktree nsd.yml");

    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let staged = policy::resolve(&repo, None, &base, Candidate::Index(&index_snapshot))
        .expect("the index candidate resolves");
    assert!(
        staged.diagnostics.is_empty(),
        "--staged reads the index, which still matches the base"
    );

    let worktree = policy::resolve(&repo, None, &base, Candidate::Worktree(&worktree_snapshot))
        .expect("the worktree candidate resolves");
    assert_eq!(
        worktree.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }],
        "the worktree copy has diverged from the base"
    );
}

#[test]
fn test_unborn_repository_uses_builtin_policy() {
    let (_dir, repo) = common::init_repo();
    let base = CommitSnapshot::head_or_empty(&repo).expect("resolve the empty-tree base");
    assert!(
        base.entries.is_empty(),
        "an unborn HEAD resolves to the empty tree"
    );

    stage_bytes(&repo, b"nsd.yml", MODE_REGULAR, b"version: 1\n");
    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");

    let resolution = policy::resolve(&repo, None, &base, Candidate::Index(&index_snapshot))
        .expect("an unborn base plus a staged config resolves");

    assert_eq!(resolution.config, Config::default());
    assert_eq!(resolution.source, ConfigSource::BuiltInDefaults);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }]
    );
}

/// A4 (Codex's scenario): under a trusted config, an unreadable
/// (over-ceiling) base must not swallow the candidate diff — its tree
/// entry still carries a blob oid, so the diff falls back to comparing
/// object ids, and a changed, invalid candidate still carries both codes.
#[test]
fn test_trusted_mode_unreadable_base_and_changed_invalid_candidate_reports_c101_and_c102() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 2\n".to_vec())],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a trusted config resolves even when the base is unreadable and the candidate changed");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(
        resolution.diagnostics,
        vec![
            Diagnostic {
                code: CODE_CONFIG_CHANGED
            },
            Diagnostic {
                code: CODE_INVALID_CONFIG
            },
        ],
        "the change must be visible (C101) and the shape failure reported (C102), even though \
         the base itself could not be read"
    );
}

/// A4: the same unreadable-base fallback, but the changed candidate is
/// shape-valid — only C101 is reported, since only an invalid candidate
/// also carries C102.
#[test]
fn test_trusted_mode_unreadable_base_and_changed_valid_candidate_reports_c101() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(
        &repo,
        &[(
            b"nsd.yml".to_vec(),
            MODE_REGULAR,
            b"version: 1\nmeasurement:\n  min_clone_lines: 5\n".to_vec(),
        )],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a trusted config resolves even when the base is unreadable and the candidate changed");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }],
        "a shape-valid changed candidate must not carry C102"
    );
}

/// A4: the worktree candidate path, which has no `Entry.oid` of its own
/// (D2) and so falls back to `Oid::hash_object` over its already-read
/// bytes, compared against the base's tree-entry oid.
#[test]
fn test_trusted_mode_unreadable_base_with_worktree_candidate_reports_the_change() {
    let (dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    sync_index_to_commit(&repo, base_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    std::fs::write(dir.path().join("nsd.yml"), b"version: 1\n")
        .expect("write a diverging worktree nsd.yml");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Worktree(&worktree_snapshot),
    )
    .expect("a trusted config resolves even when the base is unreadable and the worktree candidate changed");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }],
        "the worktree candidate's hash_object oid must differ from the base's tree-entry oid"
    );
}

/// A4, triage-ws1-r1.md row 3: the oid-equality "unchanged" shortcut must
/// not apply across a kind change. A base `nsd.yml` symlink whose target
/// is byte-identical to a candidate regular file's content shares the same
/// blob oid, but the shape did change (symlink to regular file), so it
/// must still be reported.
#[test]
fn test_trusted_mode_symlink_base_replaced_by_identical_content_reports_c101() {
    let (_dir, repo) = common::init_repo();
    let target = b"version: 1\n";
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_SYMLINK, target.to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, target.to_vec())],
    );
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect("a trusted config resolves even when the base is a symlink");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert_eq!(
        resolution.diagnostics,
        vec![Diagnostic {
            code: CODE_CONFIG_CHANGED
        }],
        "a symlink base replaced by a byte-identical regular file must still be reported as \
         changed, even though both entries share the same blob oid"
    );
}

/// triage-ws1-r2.md row 1: an **unchanged** symlink `nsd.yml` (same kind,
/// same oid) must still resolve to no diagnostics at all, the same as an
/// unchanged regular file. The round-2 fix for row 3 above over-corrected
/// by requiring both sides to be `Regular`/`Executable` before the
/// oid-equality shortcut applies, so a base and candidate that are both
/// (the same) symlink fell through to `[C101, C102]` on every trusted-mode
/// run — this is what "an unchanged candidate (equal oids) emits no C101"
/// (A4) actually requires.
#[test]
fn test_trusted_mode_unchanged_symlink_base_reports_nothing() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_SYMLINK, b"version: 1\n".to_vec())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&base),
    )
    .expect("a trusted config resolves even when the base is a symlink");

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert!(
        resolution.diagnostics.is_empty(),
        "an unchanged symlink base compared against itself must not report a change, even \
         though a symlink's oid identifies its target bytes rather than its own"
    );
}

/// Ledger row 62: the worktree candidate's early return must also see an
/// unchanged symlink `nsd.yml` (a worktree entry carries no oid, so the
/// commit/index comparison alone never matches it).
#[test]
fn test_trusted_mode_unchanged_symlink_base_with_worktree_candidate_reports_nothing() {
    let (dir, repo) = common::init_repo();
    let target = b"version: 1\n";
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_SYMLINK, target.to_vec())],
    );
    sync_index_to_commit(&repo, commit_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    std::os::unix::fs::symlink(
        std::str::from_utf8(target).expect("utf-8 target"),
        dir.path().join("nsd.yml"),
    )
    .expect("create the worktree symlink");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Worktree(&worktree_snapshot),
    )
    .expect("a trusted config resolves even when the base is a symlink");

    assert!(
        resolution.diagnostics.is_empty(),
        "an unchanged symlink nsd.yml in the worktree must not report a change"
    );
}

/// Ledger row 62, oid side: a worktree symlink `nsd.yml` whose target differs
/// from the symlink base is a changed candidate, so the early return must
/// compare oids and not only kinds.
#[test]
fn test_trusted_mode_changed_symlink_base_with_worktree_candidate_reports_c101_c102() {
    let (dir, repo) = common::init_repo();
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_SYMLINK, b"version: 1\n".to_vec())],
    );
    sync_index_to_commit(&repo, commit_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    std::os::unix::fs::symlink("other.yml", dir.path().join("nsd.yml"))
        .expect("create the retargeted worktree symlink");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Worktree(&worktree_snapshot),
    )
    .expect("a trusted config resolves even when the base is a symlink");

    let codes: Vec<_> = resolution.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG]);
}

/// The worktree twin of the kind gate: a regular base whose blob is missing
/// from the object database (so its bytes are unreadable and the diff falls
/// back to oids) shares its oid with a worktree symlink whose target bytes
/// are the same, but not its kind, so the worktree candidate must still
/// report `[C101, C102]`.
#[test]
fn test_trusted_mode_unreadable_regular_base_replaced_by_identical_worktree_symlink_reports_c101_c102(
) {
    let (dir, repo) = common::init_repo();
    let target = b"version: 1\n";
    let commit_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, target.to_vec())],
    );
    sync_index_to_commit(&repo, commit_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");
    let blob = Oid::hash_object(ObjectType::Blob, target).expect("hash the blob");
    let blob_hex = blob.to_string();
    let loose = dir
        .path()
        .join(".git/objects")
        .join(&blob_hex[..2])
        .join(&blob_hex[2..]);
    std::fs::remove_file(&loose).expect("remove the loose base blob");
    std::os::unix::fs::symlink(
        std::str::from_utf8(target).expect("utf-8 target"),
        dir.path().join("nsd.yml"),
    )
    .expect("create the worktree symlink");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Worktree(&worktree_snapshot),
    )
    .expect("a trusted config resolves even when the base blob is missing");

    assert_eq!(
        resolution.diagnostics,
        vec![
            Diagnostic {
                code: CODE_CONFIG_CHANGED
            },
            Diagnostic {
                code: CODE_INVALID_CONFIG
            }
        ],
        "a regular base replaced by a byte-identical worktree symlink is a kind change"
    );
}

/// triage-ws1-r2.md row 2: the candidate-side half of the kind gate. A
/// regular base whose bytes are unreadable (over-ceiling), compared
/// against a candidate whose `nsd.yml` is a symlink with byte-identical
/// content, must still report a change — a kind change is a change in
/// either direction. Only asserts `CODE_CONFIG_CHANGED` is present (not the
/// full vector), since whether `CODE_INVALID_CONFIG` also appears depends
/// on `fetch_candidate_bytes` reading the symlink candidate, not on this
/// gate.
#[test]
fn test_trusted_mode_regular_base_replaced_by_identical_symlink_reports_c101() {
    let (_dir, repo) = common::init_repo();
    let content = oversized_valid_config();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, content.clone())],
    );
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    common::commit_entries(&repo, &[(b"nsd.yml".to_vec(), MODE_SYMLINK, content)]);
    let candidate = CommitSnapshot::head_or_empty(&repo).expect("snapshot candidate commit");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Commit(&candidate),
    )
    .expect(
        "a trusted config resolves even when the base is unreadable and the candidate is a symlink",
    );

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert!(
        resolution.diagnostics.contains(&Diagnostic {
            code: CODE_CONFIG_CHANGED
        }),
        "a regular base replaced by a byte-identical symlink candidate must still be reported as \
         changed, even though both entries share the same blob oid"
    );
}

/// triage-ws1-r1.md row 1: no test previously reached the `Candidate::Index`
/// arm of `BaseIdentity::Oid`, so the mutant `.and(None)` (dropping the
/// index candidate's own oid to `None`) survived: an unchanged oversized
/// index candidate would then read as a spurious change.
#[test]
fn test_trusted_mode_unreadable_base_with_unchanged_index_candidate_reports_nothing() {
    let (_dir, repo) = common::init_repo();
    let base_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, oversized_valid_config())],
    );
    sync_index_to_commit(&repo, base_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    let index_snapshot = IndexSnapshot::open(&repo).expect("open index snapshot");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Index(&index_snapshot),
    )
    .expect(
        "a trusted config resolves even when the base is unreadable and the index candidate is \
         unchanged",
    );

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert!(
        resolution.diagnostics.is_empty(),
        "an unchanged index candidate, compared by oid against an unreadable base, must not \
         report a change"
    );
}

/// triage-ws1-r1.md row 2: the only worktree oid-comparison test previously
/// asserted a *difference*, so the mutant `Blob` -> `Tree` in the
/// `Oid::hash_object` call survived: an unchanged worktree candidate,
/// compared against a base whose blob is missing from the ODB (a
/// Git-domain read failure, not a shape one), would then read as a
/// spurious change.
#[test]
fn test_trusted_mode_missing_base_blob_with_identical_worktree_candidate_reports_nothing() {
    let (dir, repo) = common::init_repo();
    let content = b"version: 1\n";
    let base_oid = common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, content.to_vec())],
    );
    sync_index_to_commit(&repo, base_oid);
    let base = CommitSnapshot::head_or_empty(&repo).expect("snapshot base commit");

    std::fs::write(dir.path().join("nsd.yml"), content)
        .expect("write an identical worktree nsd.yml");
    let worktree_snapshot = WorktreeSnapshot::open(&repo).expect("open worktree snapshot");

    let blob_oid = Oid::hash_object(ObjectType::Blob, content)
        .expect("hash the fixture content the same way the base blob was written");
    let blob_hex = blob_oid.to_string();
    let object_path = repo
        .path()
        .join("objects")
        .join(&blob_hex[..2])
        .join(&blob_hex[2..]);
    std::fs::remove_file(&object_path).expect("delete the loose nsd.yml blob from the ODB");

    let trusted_dir = tempfile::TempDir::new().expect("create a temp dir for the trusted config");
    let trusted_path = trusted_dir.path().join("trusted.yml");
    std::fs::write(&trusted_path, b"version: 1\n").expect("write trusted config");

    let resolution = policy::resolve(
        &repo,
        Some(trusted_path.as_path()),
        &base,
        Candidate::Worktree(&worktree_snapshot),
    )
    .expect(
        "a trusted config resolves even when the base blob is missing from the ODB and the \
         worktree candidate is unchanged",
    );

    assert_eq!(resolution.source, ConfigSource::Trusted);
    assert!(
        resolution.diagnostics.is_empty(),
        "an identical worktree candidate, hashed to the same oid as the base's tree entry, must \
         not report a change"
    );
}

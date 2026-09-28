//! Strict repository-root `nsd.yml` parsing (WS-3, `C102`).

mod common;

use nsd::config::{Config, ConfigError, Severity, CODE_INVALID_CONFIG};
use nsd::git::snapshot::CommitSnapshot;
use nsd::model::DEFAULT_MIN_CLONE_LINES;

/// The verbatim example from `nsd-plan-implementation.md`'s *Public
/// contracts* section.
const SPEC_EXAMPLE: &str = "\
version: 1
include: [src/**]       # omitted means every supported source path
exclude: [vendor/**]

measurement:
  min_clone_lines: 10   # any positive integer; enters the fingerprint and cache key (A2)

policy:
  NSD-E101: deny
  NSD-E102: deny
  NSD-V101: deny
  NSD-V102: deny
  NSD-S102: warn

output:
  max_terminal_diagnostics: 50
  max_agent_diagnostics: 30
";

/// This suite's own fixture constant (D22): a regular file's Git mode,
/// used only here.
const MODE_REGULAR: i32 = 0o100644;

fn assert_is_c102(result: Result<Config, ConfigError>, needle: &str) {
    let err = result.expect_err("expected an NSD-C102 error");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
    assert!(
        err.to_string().contains(needle),
        "expected {needle:?} in the message, got: {err}"
    );
}

#[test]
fn spec_example_parses() {
    let config = Config::parse(SPEC_EXAMPLE.as_bytes()).expect("the spec example is valid");
    assert_eq!(config.version, 1);
    assert_eq!(config.include, Some(vec!["src/**".to_string()]));
    assert_eq!(config.exclude, vec!["vendor/**".to_string()]);
    assert_eq!(config.measurement.min_clone_lines, 10);
    assert_eq!(config.policy.nsd_e101, Severity::Deny);
    assert_eq!(config.policy.nsd_e102, Severity::Deny);
    assert_eq!(config.policy.nsd_v101, Severity::Deny);
    assert_eq!(config.policy.nsd_v102, Severity::Deny);
    assert_eq!(config.policy.nsd_s102, Severity::Warn);
    assert_eq!(config.output.max_terminal_diagnostics, 50);
    assert_eq!(config.output.max_agent_diagnostics, 30);
}

#[test]
fn omitted_sections_take_defaults() {
    let config = Config::parse(b"version: 1\n").expect("a bare version is valid");
    assert_eq!(config.include, None);
    assert_eq!(config.exclude, Vec::<String>::new());
    assert_eq!(config.measurement.min_clone_lines, DEFAULT_MIN_CLONE_LINES);
    assert_eq!(config.policy.nsd_e101, Severity::Deny);
    assert_eq!(config.policy.nsd_e102, Severity::Deny);
    assert_eq!(config.policy.nsd_v101, Severity::Deny);
    assert_eq!(config.policy.nsd_v102, Severity::Deny);
    assert_eq!(config.policy.nsd_s102, Severity::Warn);
    assert_eq!(config.output.max_terminal_diagnostics, 50);
    assert_eq!(config.output.max_agent_diagnostics, 30);
}

#[test]
fn unknown_fields_are_c102() {
    assert_is_c102(Config::parse(b"version: 1\nfoo: bar\n"), "foo");
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  foo: 1\n"),
        "foo",
    );
}

#[test]
fn duplicate_fields_are_c102() {
    assert_is_c102(Config::parse(b"version: 1\nversion: 1\n"), "version");
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: 1\n  min_clone_lines: 2\n"),
        "min_clone_lines",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E101: deny\n  NSD-E101: warn\n"),
        "NSD-E101",
    );
}

#[test]
fn bad_version_is_c102() {
    assert_is_c102(Config::parse(b"version: 2\n"), "version 2");
    assert_is_c102(Config::parse(b"version: \"1\"\n"), "version");
    assert_is_c102(Config::parse(b"include: [src/**]\n"), "version");
}

#[test]
fn unsupported_codes_are_c102() {
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-S101: deny\n"),
        "NSD-S101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-A101: deny\n"),
        "NSD-A101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-X999: deny\n"),
        "NSD-X999",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  E101: deny\n"),
        "E101",
    );
}

#[test]
fn bad_severity_is_c102() {
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E101: error\n"),
        "NSD-E101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E101: Deny\n"),
        "NSD-E101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E101: true\n"),
        "NSD-E101",
    );
}

#[test]
fn bad_globs_are_c102() {
    assert_is_c102(
        Config::parse(b"version: 1\ninclude: [\"src/[\"]\n"),
        "src/[",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nexclude: [\"src/[\"]\n"),
        "src/[",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nexclude: [\"#vendor/**\"]\n"),
        "#vendor/**",
    );
}

#[test]
fn negated_patterns_are_c102() {
    assert_is_c102(
        Config::parse(b"version: 1\ninclude: [\"!vendor/**\"]\n"),
        "!vendor/**",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nexclude: [\"!vendor/**\"]\n"),
        "!vendor/**",
    );
}

#[test]
fn non_positive_min_clone_lines_is_c102() {
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: 0\n"),
        "min_clone_lines",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: -3\n"),
        "min_clone_lines",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: \"10\"\n"),
        "min_clone_lines",
    );
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: 1.5\n"),
        "min_clone_lines",
    );
}

#[test]
fn empty_include_is_c102() {
    assert_is_c102(Config::parse(b"version: 1\ninclude: []\n"), "include");
    assert_is_c102(
        Config::parse(b"version: 1\ninclude: [\"\"]\n"),
        "empty or comment-only",
    );
    assert_is_c102(
        Config::parse(b"version: 1\ninclude: [\"   \"]\n"),
        "empty or comment-only",
    );
    assert_is_c102(
        Config::parse(b"version: 1\ninclude: [\"#src/**\"]\n"),
        "#src/**",
    );
    assert_is_c102(Config::parse(b"version: 1\ninclude:\n"), "include");
    assert_is_c102(Config::parse(b"version: 1\ninclude: ~\n"), "include");
    assert_is_c102(Config::parse(b"version: 1\ninclude: null\n"), "include");
}

#[test]
fn non_utf8_and_multi_document_are_c102() {
    let mut non_utf8 = b"version: 1\n# ".to_vec();
    non_utf8.push(0xFF);
    non_utf8.push(b'\n');
    assert_is_c102(Config::parse(&non_utf8), "UTF-8");

    assert_is_c102(
        Config::parse(b"version: 1\n---\nversion: 1\n"),
        "more than one document",
    );
}

#[test]
fn root_nsd_yml_only() {
    let (_dir, repo) = common::init_repo();

    // `sub/nsd.yml` and `NSD.yml` are not the repository-root `nsd.yml`:
    // built-in defaults apply. Each decoy carries its own distinct
    // `min_clone_lines` value, so a loader that picked either one instead
    // of finding nothing would produce a value other than the default.
    common::commit_entries(
        &repo,
        &[
            (
                b"sub/nsd.yml".to_vec(),
                MODE_REGULAR,
                b"version: 1\nmeasurement:\n  min_clone_lines: 77\n".to_vec(),
            ),
            (
                b"NSD.yml".to_vec(),
                MODE_REGULAR,
                b"version: 1\nmeasurement:\n  min_clone_lines: 88\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let config = nsd::config::load_from_commit(&repo, &snapshot).expect("defaults expected");
    assert_eq!(config.measurement.min_clone_lines, DEFAULT_MIN_CLONE_LINES);
    assert_eq!(
        config,
        Config::parse(b"version: 1\n").expect("defaults"),
        "a decoy at sub/nsd.yml or NSD.yml must not change any part of the default config"
    );

    // A root `nsd.yml` is discovered and parsed.
    common::commit_entries(
        &repo,
        &[
            (
                b"sub/nsd.yml".to_vec(),
                MODE_REGULAR,
                b"version: 1\nmeasurement:\n  min_clone_lines: 77\n".to_vec(),
            ),
            (
                b"NSD.yml".to_vec(),
                MODE_REGULAR,
                b"version: 1\nmeasurement:\n  min_clone_lines: 88\n".to_vec(),
            ),
            (
                b"nsd.yml".to_vec(),
                MODE_REGULAR,
                b"version: 1\nmeasurement:\n  min_clone_lines: 25\n".to_vec(),
            ),
        ],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let config = nsd::config::load_from_commit(&repo, &snapshot).expect("root file expected");
    assert_eq!(config.measurement.min_clone_lines, 25);

    // An invalid root `nsd.yml` is `NSD-C102`.
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 2\n".to_vec())],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let err = nsd::config::load_from_commit(&repo, &snapshot).expect_err("invalid root file");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
}

#[test]
fn unreadable_root_nsd_yml_keeps_g101() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"nsd.yml".to_vec(), MODE_REGULAR, b"version: 1\n".to_vec())],
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"nsd.yml")
        .expect("nsd.yml entry present in the snapshot");
    let blob_oid = entry.oid.expect("a regular file entry carries a blob oid");
    let hex = blob_oid.to_string();
    let object_path = repo.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    std::fs::remove_file(&object_path).expect("remove the loose blob object from the ODB");

    let err =
        nsd::config::load_from_commit(&repo, &snapshot).expect_err("blob is gone from the ODB");
    assert_eq!(err.code(), nsd::git::CODE_SNAPSHOT_UNAVAILABLE);
    assert!(err.to_string().starts_with("[NSD-G101]"), "{err}");
}

#[test]
fn null_values_are_c102() {
    // `version` and `exclude: ~` are already C102 today (regression pins:
    // `version` is a plain `u32`, `exclude: ~` is rejected by `Vec`
    // deserialization); every other case here is a red case before D30.
    assert_is_c102(Config::parse(b"version: ~\n"), "version");
    assert_is_c102(Config::parse(b"version: 1\nexclude: ~\n"), "exclude");
    assert_is_c102(Config::parse(b"version: 1\nexclude:\n"), "exclude");

    assert_is_c102(Config::parse(b"version: 1\nmeasurement:\n"), "measurement");
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement: null\n"),
        "measurement",
    );

    assert_is_c102(Config::parse(b"version: 1\npolicy: ~\n"), "policy");
    assert_is_c102(Config::parse(b"version: 1\npolicy:\n"), "policy");
    assert_is_c102(Config::parse(b"version: 1\noutput: ~\n"), "output");
    assert_is_c102(Config::parse(b"version: 1\noutput:\n"), "output");

    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: ~\n"),
        "min_clone_lines",
    );

    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E101: ~\n"),
        "NSD-E101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-E102: null\n"),
        "NSD-E102",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-V101: Null\n"),
        "NSD-V101",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-V102: NULL\n"),
        "NSD-V102",
    );
    assert_is_c102(
        Config::parse(b"version: 1\npolicy:\n  NSD-S102: ~\n"),
        "NSD-S102",
    );

    assert_is_c102(
        Config::parse(b"version: 1\noutput:\n  max_terminal_diagnostics: ~\n"),
        "max_terminal_diagnostics",
    );
    assert_is_c102(
        Config::parse(b"version: 1\noutput:\n  max_agent_diagnostics: null\n"),
        "max_agent_diagnostics",
    );
}

#[test]
fn empty_tagged_scalars_are_c102() {
    // An explicit tag on an empty plain scalar is not a null spelling, but
    // serde_yaml_ng still turns it into an empty sequence or mapping on a
    // container key, which would silently default the key.
    assert_is_c102(Config::parse(b"version: 1\nexclude: !!seq\n"), "exclude");
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement: !!map\n"),
        "measurement",
    );
    assert_is_c102(Config::parse(b"version: 1\npolicy: !!str\n"), "policy");
    assert_is_c102(Config::parse(b"version: 1\noutput: !!map\n"), "output");
    assert_is_c102(Config::parse(b"version: 1\nexclude: !custom\n"), "exclude");
    // `!!null` on an empty scalar already fails; the error must name the key.
    assert_is_c102(
        Config::parse(b"version: 1\nmeasurement: !!null\n"),
        "measurement",
    );

    let config = Config::parse(b"version: 1\nexclude: !!seq []\npolicy: !!map {}\n")
        .expect("a tagged, non-empty container is not an empty scalar");
    assert_eq!(config.exclude, Vec::<String>::new());
}

#[test]
fn empty_mappings_and_empty_exclude_still_default() {
    let config =
        Config::parse(b"version: 1\nmeasurement: {}\npolicy: {}\noutput: {}\nexclude: []\n")
            .expect("empty mappings and an empty exclude list are not null");
    assert_eq!(config.exclude, Vec::<String>::new());
    assert_eq!(config.measurement.min_clone_lines, DEFAULT_MIN_CLONE_LINES);
    assert_eq!(config.policy.nsd_e101, Severity::Deny);
    assert_eq!(config.policy.nsd_e102, Severity::Deny);
    assert_eq!(config.policy.nsd_v101, Severity::Deny);
    assert_eq!(config.policy.nsd_v102, Severity::Deny);
    assert_eq!(config.policy.nsd_s102, Severity::Warn);
    assert_eq!(config.output.max_terminal_diagnostics, 50);
    assert_eq!(config.output.max_agent_diagnostics, 30);
}

#[test]
fn min_clone_lines_positive_value_exposed() {
    let config = Config::parse(b"version: 1\nmeasurement:\n  min_clone_lines: 25\n")
        .expect("a valid config");
    assert_eq!(config.measurement.min_clone_lines, 25);
}

/// A1: runs `f` on a detached, `'static` thread and waits at most 5s for its
/// result (same pattern as `tests/git_diff.rs:1122-1136`'s `with_timeout`,
/// duplicated here since each `tests/*.rs` file is its own compilation
/// unit). A hang inside `f` (the defect this guards against: `load_trusted`
/// calling `File::open` on a FIFO with no writer) leaves the thread running
/// forever, but the test process still exits: nothing here ever joins it.
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

/// A1: a trusted `--config` path naming a FIFO with no writer must not
/// block `File::open` — it must return `NSD-C102` promptly instead.
#[test]
#[cfg(unix)]
fn test_load_trusted_fifo_is_c102_without_blocking() {
    let dir = tempfile::TempDir::new().expect("create a temp dir for the fifo case");
    let fifo_path = dir.path().join("trusted.yml");
    let status = std::process::Command::new("/usr/bin/mkfifo")
        .arg(&fifo_path)
        .status()
        .expect("spawn mkfifo for the trusted config path");
    assert!(status.success(), "mkfifo must succeed");

    let result = with_timeout("load_trusted on a FIFO with no writer", move || {
        nsd::config::load_trusted(&fifo_path)
    });
    let err = result.expect_err("a FIFO trusted config path must not load");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
}

/// A1: a trusted `--config` path naming a directory is `NSD-C102`, the same
/// as a missing or over-ceiling one.
#[test]
fn test_load_trusted_directory_is_c102() {
    let dir = tempfile::TempDir::new().expect("create a temp dir for the directory case");
    let err = nsd::config::load_trusted(dir.path()).expect_err("a directory must not load");
    assert_eq!(err.code(), CODE_INVALID_CONFIG);
}

/// A1: unlike a repository entry's never-follow `symlink_metadata` check, a
/// trusted `--config` path is operator-chosen, so a symlink to a regular
/// file must still load.
#[test]
#[cfg(unix)]
fn test_load_trusted_follows_a_symlink_to_a_regular_file() {
    let dir = tempfile::TempDir::new().expect("create a temp dir for the symlink case");
    let real_path = dir.path().join("real.yml");
    std::fs::write(
        &real_path,
        b"version: 1\nmeasurement:\n  min_clone_lines: 42\n",
    )
    .expect("write the real trusted config");
    let link_path = dir.path().join("trusted.yml");
    std::os::unix::fs::symlink(&real_path, &link_path).expect("create the symlink");

    let config = nsd::config::load_trusted(&link_path)
        .expect("a symlink to a regular trusted config file must still load");
    assert_eq!(config.measurement.min_clone_lines, 42);
}

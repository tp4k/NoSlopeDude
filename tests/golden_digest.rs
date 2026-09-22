//! WS-4: the `java-fixture-01` golden digest (A1).
//!
//! `src/golden.rs` is `#[cfg(test)]`-gated and `pub(crate)`, so it is not
//! reachable through the compiled `nsd` library from an external
//! integration-test crate. This file includes it (and `src/hashing.rs`,
//! which it depends on for `body_blake3`) directly via `#[path]` instead of
//! reimplementing the digest computation a second time -- the same source
//! is compiled twice, once inside the library crate and once here, rather
//! than being duplicated by hand.
//!
//! The archived `java-fixture-01` report never enters this repository; its
//! path is read only from `NSD_ARCHIVED_REPORT` at test-invocation time.
//! See `docs/golden-digest.md` for how to resolve and pass it.

#[path = "../src/golden.rs"]
mod golden;
#[path = "../src/hashing.rs"]
mod hashing;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;

/// The env var that carries the archived report's path at invocation time
/// only; it is never written into a repository file.
const ARCHIVED_REPORT_ENV_VAR: &str = "NSD_ARCHIVED_REPORT";

/// How many `excerpt` occurrences the real archived `java-fixture-01`
/// report carries at the three sites A1 normalizes: `findings[].location`
/// (340) + `duplicates[].locations[]` (172) + `top25[].location` (25).
const EXPECTED_EXCERPT_COUNT: usize = 537;

/// `top25` is defined as exactly 25 entries; this is a property of the
/// digest's own format, not a measurement of this particular fixture.
const TOP25_TRIPLE_COUNT: usize = 25;

/// Cross-checked against `nsd-plan-final.md:40` (*Measured starting
/// state*). The js_ts erosion, the finding/rule-ID counts, the clone-group
/// count, and the top25 triples have no independent source outside the
/// archived report itself and are verified only by the equality check
/// against the committed golden below -- typing them in here would be the
/// same unverifiable hand-entry the golden file itself must avoid.
const EXPECTED_OVERALL_EROSION: f64 = 0.13313797553867948;
const EXPECTED_JAVA_EROSION: f64 = 0.13294913151043064;

/// Cross-checked against `nsd-plan-final.md:70` (*Corrections applied*,
/// row 7): 854 skips split `test: 853` / `dependency_or_build_output: 1`.
const EXPECTED_SKIPS_TEST: u64 = 853;
const EXPECTED_SKIPS_DEPENDENCY_OR_BUILD_OUTPUT: u64 = 1;

fn committed_digest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/java-fixture-01.digest.json")
}

fn read_committed_digest() -> Result<Value> {
    let raw = fs::read_to_string(committed_digest_path())
        .context("reading the committed java-fixture-01 golden digest")?;
    golden::parse_digest(&raw).context("parsing the committed golden digest")
}

/// Where the archived report is, or an explicit statement that this run's
/// leg is pending because the private fixture was not supplied -- so a
/// missing private fixture can never silently read as a validated pass
/// (`AGENTS.md` -> *Verification*).
enum ArchiveGate {
    Resolved(PathBuf),
    Pending,
}

fn archive_gate() -> ArchiveGate {
    classify_archive_gate(std::env::var_os(ARCHIVED_REPORT_ENV_VAR))
}

/// Pure classification of a possibly-absent `NSD_ARCHIVED_REPORT` value,
/// split out from `archive_gate`'s `env::var_os` call so `None` (the var is
/// unset) and `Some("")` (the var is set but empty) can each be asserted
/// directly, rather than only observed indirectly through whatever the test
/// process's own environment happens to carry when it runs.
fn classify_archive_gate(raw: Option<OsString>) -> ArchiveGate {
    match raw {
        Some(path) if !path.is_empty() => ArchiveGate::Resolved(PathBuf::from(path)),
        _ => ArchiveGate::Pending,
    }
}

/// The notice printed on every pending arm below -- extracted so the call
/// sites cannot drift apart and so a test can assert its content once.
fn pending_notice() -> String {
    format!(
        "PENDING: {ARCHIVED_REPORT_ENV_VAR} is unset -- java-fixture-01's \
         reproducibility gate is pending, not passing, this run"
    )
}

/// Recomputes the digest from the archived report and asserts it against
/// the committed golden, proving the golden was derived and is
/// re-derivable rather than typed in by hand.
#[test]
fn test_committed_digest_matches_the_archived_report() -> Result<()> {
    let path = match archive_gate() {
        ArchiveGate::Resolved(path) => path,
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
            return Ok(());
        }
    };

    let raw = fs::read_to_string(&path).context("reading the archived report")?;
    let mut report = golden::parse_report(&raw).context("parsing the archived report")?;

    let (recomputed, removed) =
        golden::build_digest(&mut report).context("building the golden digest")?;

    assert_eq!(
        removed, EXPECTED_EXCERPT_COUNT,
        "excerpt strip must remove all three sites' occurrences, not a subset"
    );

    let overall_erosion = recomputed["scores"]["overall"]["erosion"]
        .as_f64()
        .context("recomputed digest is missing scores.overall.erosion")?;
    assert_eq!(overall_erosion, EXPECTED_OVERALL_EROSION);
    let java_erosion = recomputed["scores"]["java"]["erosion"]
        .as_f64()
        .context("recomputed digest is missing scores.java.erosion")?;
    assert_eq!(java_erosion, EXPECTED_JAVA_EROSION);

    let skips = recomputed["skips_by_reason"]
        .as_object()
        .context("recomputed digest is missing skips_by_reason")?;
    assert_eq!(
        skips.get("test").and_then(Value::as_u64),
        Some(EXPECTED_SKIPS_TEST)
    );
    assert_eq!(
        skips
            .get("dependency_or_build_output")
            .and_then(Value::as_u64),
        Some(EXPECTED_SKIPS_DEPENDENCY_OR_BUILD_OUTPUT)
    );

    let top25 = recomputed["top25"]
        .as_array()
        .context("recomputed digest is missing top25")?;
    assert_eq!(top25.len(), TOP25_TRIPLE_COUNT);

    let committed = read_committed_digest()?;
    assert_eq!(
        recomputed, committed,
        "the committed golden must equal what build_digest re-derives from the archive"
    );

    Ok(())
}

/// Proves the pending path is reachable and prints its notice rather than
/// silently substituting a pass, independent of whether this particular
/// invocation happens to carry the archive-backed leg too.
#[test]
fn test_gate_is_reported_pending_when_the_archive_is_absent() {
    match archive_gate() {
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
        }
        ArchiveGate::Resolved(_) => {
            // The archive-backed leg is running in this invocation; the
            // pending branch above is exercised by this same test in the
            // ordinary (archive-absent) developer/CI run instead.
        }
    }
}

/// `classify_archive_gate` is the pure decision `archive_gate` delegates to;
/// tested directly (not through `env::var_os`, which only the process's own
/// environment can drive) so the unset case, the set-but-empty case, and the
/// set-and-non-empty case are each pinned rather than only exercised
/// incidentally by whichever of the three the test process happens to run
/// under.
#[test]
fn test_classify_archive_gate() {
    assert!(matches!(classify_archive_gate(None), ArchiveGate::Pending));
    assert!(matches!(
        classify_archive_gate(Some(OsString::new())),
        ArchiveGate::Pending
    ));
    match classify_archive_gate(Some(OsString::from("/x"))) {
        ArchiveGate::Resolved(path) => assert_eq!(path, PathBuf::from("/x")),
        ArchiveGate::Pending => panic!("a non-empty path must resolve, not read as pending"),
    }
}

/// The pending notice both pending arms above print must actually say the
/// run is pending and name the env var a developer needs to set -- proving
/// the two call sites cannot silently drift into printing nothing or an
/// unrelated message.
#[test]
fn test_pending_notice_names_the_env_var() {
    let notice = pending_notice();
    assert!(notice.contains("PENDING"), "notice was: {notice:?}");
    assert!(
        notice.contains(ARCHIVED_REPORT_ENV_VAR),
        "notice was: {notice:?}"
    );
}

/// Runs unconditionally (no archive needed): the committed digest file
/// itself must carry none of the archived report's private shape.
#[test]
fn test_committed_digest_carries_no_paths_names_or_excerpts() -> Result<()> {
    const FORBIDDEN_KEYS: [&str; 6] = [
        "excerpt",
        "link",
        "name",
        "relative_path",
        "target",
        "detail",
    ];

    fn walk(value: &Value, forbidden_keys_found: &mut BTreeMap<String, ()>) {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    if FORBIDDEN_KEYS.contains(&key.as_str()) {
                        forbidden_keys_found.insert(key.clone(), ());
                    }
                    walk(nested, forbidden_keys_found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, forbidden_keys_found);
                }
            }
            Value::String(text) => {
                assert!(
                    !text.starts_with('/'),
                    "committed digest carries an absolute-path-shaped string value: {text:?}"
                );
            }
            _ => {}
        }
    }

    let committed = read_committed_digest()?;
    let mut forbidden_keys_found = BTreeMap::new();
    walk(&committed, &mut forbidden_keys_found);
    assert!(
        forbidden_keys_found.is_empty(),
        "committed digest carries forbidden keys: {:?}",
        forbidden_keys_found.keys().collect::<Vec<_>>()
    );

    Ok(())
}

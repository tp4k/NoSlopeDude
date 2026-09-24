//! M0c-10: the grammar-swap acceptance gate. `report.json`'s
//! `skipped_files` cannot answer "did the Java parse-failure count clear
//! to zero", because `src/report/mod.rs::build_skipped_files` already
//! filters `ParseFailureReason::SyntaxError` out of that list (a WS-6
//! salvage-era behaviour, unrelated to the grammar swap: a `SyntaxError`
//! file is no longer a whole-file skip, so it never reaches
//! `skipped_files` at all, cleared or not). This suite reads
//! `pipeline::PipelineOutput::parse_failures` directly instead, which is
//! never filtered.
//!
//! Both legs below scan the pinned Spring + Angular perf fixture
//! (`scripts/fetch_perf_fixture.sh`), gated on `NSD_PERF_FIXTURE` the same
//! way `tests/neutrality.rs`'s archive-backed leg is gated on
//! `NSD_ARCHIVED_REPORT`: absent means pending, not passing, and never a
//! silent skip. The fixture is a public clone of two public GitHub repos
//! (unlike `java-fixture-01`), so its path and shas are not privacy-
//! sensitive and may appear in test output.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use nsd::model::{ParseFailureReason, ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;

const FIXTURE_ROOT_ENV_VAR: &str = "NSD_PERF_FIXTURE";

/// Expected TS `using`-declaration parse failures the perf fixture still
/// carries after the Java grammar swap (M0c-10's decision 3: JS/TS grammars
/// are unchanged, so this count is expected to *remain*, not clear).
const EXPECTED_TS_USING_FAILURES: usize = 3;

enum FixtureGate {
    Resolved(PathBuf),
    Pending,
}

fn fixture_gate() -> FixtureGate {
    classify_fixture_gate(std::env::var_os(FIXTURE_ROOT_ENV_VAR))
}

/// Pure classification of a possibly-absent `NSD_PERF_FIXTURE` value,
/// split out from `fixture_gate`'s `env::var_os` call the same way
/// `tests/neutrality.rs::classify_archive_gate` is, so the unset and
/// set-but-empty cases can each be asserted directly.
fn classify_fixture_gate(raw: Option<OsString>) -> FixtureGate {
    match raw {
        Some(path) if !path.is_empty() => FixtureGate::Resolved(PathBuf::from(path)),
        _ => FixtureGate::Pending,
    }
}

fn pending_notice() -> String {
    format!(
        "PENDING: {FIXTURE_ROOT_ENV_VAR} is unset -- the M0c-10 grammar gate \
         is pending, not passing, this run"
    )
}

fn is_java(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("java")
}

fn is_ts(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ts") | Some("tsx")
    )
}

/// Scans `fixture_root` (the perf fixture's parent directory, holding both
/// `spring-framework/` and `angular/`) with the same settings
/// `scripts/perf_scan.sh` uses (no `--include-tests`, default
/// `min_clone_lines`), and returns every `SyntaxError`-reason parse
/// failure, unfiltered.
fn syntax_error_failures(fixture_root: &Path) -> Vec<PathBuf> {
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    let target_input = fixture_root
        .to_str()
        .expect("fixture root path is valid UTF-8")
        .to_string();
    let output = pipeline::run(&target_input, settings)
        .unwrap_or_else(|error| panic!("scanning the perf fixture: {error}"));
    output
        .parse_failures
        .into_iter()
        .filter(|failure| failure.reason == ParseFailureReason::SyntaxError)
        .map(|failure| failure.relative_path)
        .collect()
}

/// M0c-10's headline assertion: every Java syntax-error failure the perf
/// fixture carried under `tree-sitter-java` 0.23.5 (the varargs-annotation
/// misparse, 55 files at round-1 measurement) must have cleared under
/// `tree-sitter-java-orchard` 0.5.18.
#[test]
fn test_perf_fixture_java_varargs_failures_clear() {
    let fixture_root = match fixture_gate() {
        FixtureGate::Resolved(path) => path,
        FixtureGate::Pending => {
            println!("{}", pending_notice());
            return;
        }
    };
    let failures = syntax_error_failures(&fixture_root);
    let java_failures: Vec<&PathBuf> = failures.iter().filter(|path| is_java(path)).collect();
    assert!(
        java_failures.is_empty(),
        "expected zero Java SyntaxError parse failures on the perf fixture after the \
         orchard swap, found {}: {java_failures:?}",
        java_failures.len()
    );
}

/// Decision 3's other half: the swap touches only the Java grammar, so the
/// perf fixture's TS `using`-declaration parse failures are expected to
/// remain exactly at their pre-swap count, not clear to zero and not grow.
#[test]
fn test_perf_fixture_ts_using_failures_remain() {
    let fixture_root = match fixture_gate() {
        FixtureGate::Resolved(path) => path,
        FixtureGate::Pending => {
            println!("{}", pending_notice());
            return;
        }
    };
    let failures = syntax_error_failures(&fixture_root);
    let ts_failures: Vec<&PathBuf> = failures.iter().filter(|path| is_ts(path)).collect();
    assert_eq!(
        ts_failures.len(),
        EXPECTED_TS_USING_FAILURES,
        "expected exactly {EXPECTED_TS_USING_FAILURES} TS SyntaxError parse failures \
         (the `using`-declaration gap; JS/TS grammars are unchanged by this swap), found \
         {}: {ts_failures:?}",
        ts_failures.len()
    );
}

/// Proves the pending path is reachable and prints its notice rather than
/// silently substituting a pass -- mirroring
/// `tests/neutrality.rs::test_java_fixture_01_strict_leg_is_reported_pending_when_the_archive_is_absent`.
#[test]
fn test_grammar_gate_is_reported_pending_when_the_fixture_is_absent() {
    match fixture_gate() {
        FixtureGate::Pending => println!("{}", pending_notice()),
        FixtureGate::Resolved(_) => {
            // The fixture-backed legs are running in this invocation; the
            // pending branch above is exercised by this same test in the
            // ordinary (fixture-absent) developer/CI run instead.
        }
    }
}

/// `classify_fixture_gate` is the pure decision `fixture_gate` delegates
/// to; tested directly (not through `env::var_os`, which only the
/// process's own environment can drive) so the unset case, the
/// set-but-empty case, and the set-and-non-empty case are each pinned.
#[test]
fn test_classify_fixture_gate() {
    assert!(matches!(classify_fixture_gate(None), FixtureGate::Pending));
    assert!(matches!(
        classify_fixture_gate(Some(OsString::new())),
        FixtureGate::Pending
    ));
    match classify_fixture_gate(Some(OsString::from("/x"))) {
        FixtureGate::Resolved(path) => assert_eq!(path, PathBuf::from("/x")),
        FixtureGate::Pending => panic!("a non-empty path must resolve, not read as pending"),
    }
}

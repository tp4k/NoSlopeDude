//! WS-2: the measurement-neutrality gate `nsd-plan-final.md` M0b item 8
//! requires -- scans a fixed, copied fixture corpus with fixed settings and
//! asserts the rendered `report.json` is byte-identical to a committed
//! pre-IR baseline, captured now while the tree is still pre-IR.
//!
//! This suite never shells out to git and needs no worktree, no network and
//! no private archive: `scripts/neutrality_gate.sh` (the operator-run
//! re-capture/comparison tool) is the mechanism that reaches into git
//! history, kept deliberately separate from this always-runnable gate.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use nsd::model::{ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;

/// Corpus with zero parse failures: `rules/__tests__`'s own "Clean*"
/// fixtures, already relied on by `tests/rules.rs` as clean inputs.
const CLEAN_CORPUS_SOURCES: &[&str] = &[
    "tests/fixtures/rules/__tests__/CleanJava.java",
    "tests/fixtures/rules/__tests__/CleanJs.js",
    "tests/fixtures/rules/__tests__/CleanTs.ts",
];

/// Corpus with one parse failure (`Broken.java`) alongside one file that
/// parses (`Good.java`), so metrics, clones and rules all still run.
const MALFORMED_CORPUS_SOURCES: &[&str] = &[
    "tests/fixtures/rules/broken/Broken.java",
    "tests/fixtures/rules/broken/Good.java",
];

const CLEAN_BASELINE_PATH: &str = "tests/golden/neutrality/clean.report.json";
const MALFORMED_BASELINE_PATH: &str = "tests/golden/neutrality/malformed.report.json";

/// The label `normalize` substitutes for the corpus's volatile tempdir
/// path -- the one field that can never be made constant across
/// invocations, because each invocation (including the one that captured
/// the committed baseline) copies the corpus into a fresh tempdir.
const NORMALIZED_TARGET_LABEL: &str = "<neutrality-corpus>";

/// Declared measurement deltas the malformed corpus is permitted to carry
/// against its baseline -- JSON pointers such as `/skipped_files/0/reason`.
/// Empty at this stream: WS-2 captures the pre-IR baseline with no
/// analyzer retargeted yet, so there is nothing to declare. WS-6 (salvage,
/// the `SkipReason` split) populates this list when it lands the deltas
/// M0b item 8 explicitly permits.
const DECLARED_DELTAS: &[&str] = &[];

fn manifest_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Copies `sources` (paths relative to `CARGO_MANIFEST_DIR`) flat into
/// `dest_root/src/<file name>` -- flattened, rather than mirroring each
/// source's own directory name, so a source segment such as `__tests__` or
/// `broken` never lands somewhere the D16 default exclusions would treat
/// as a test or generated-code directory and skip.
fn copy_corpus(sources: &[&str], dest_root: &Path) {
    let dest_src = dest_root.join("src");
    fs::create_dir_all(&dest_src).expect("create corpus src dir");
    for source in sources {
        let from = manifest_path(source);
        let file_name = from.file_name().expect("fixture source has a file name");
        let contents = fs::read_to_string(&from)
            .unwrap_or_else(|error| panic!("reading corpus source {from:?}: {error}"));
        fs::write(dest_src.join(file_name), contents)
            .unwrap_or_else(|error| panic!("writing copied corpus file {file_name:?}: {error}"));
    }
}

/// Copies `sources` into a fresh tempdir and scans it with fixed settings
/// -- the same tempdir-fixture / tempdir-output / explicit-`ScanSettings`
/// shape `tests/e2e_local.rs::run_scan` uses.
fn scan_corpus(sources: &[&str]) -> (tempfile::TempDir, tempfile::TempDir, Value) {
    let fixture_dir = tempfile::tempdir().expect("fixture tempdir");
    copy_corpus(sources, fixture_dir.path());
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    let target_input = fixture_dir
        .path()
        .to_str()
        .expect("fixture path is valid UTF-8")
        .to_string();
    pipeline::run(&target_input, settings).expect("pipeline run should succeed");
    let json_text =
        fs::read_to_string(output_dir.path().join("report.json")).expect("report.json exists");
    let value: Value = serde_json::from_str(&json_text).expect("valid JSON");
    (fixture_dir, output_dir, value)
}

/// Replaces `scan.target` with the fixed label and nothing else.
fn normalize(report: &Value) -> Value {
    let mut normalized = report.clone();
    let scan = normalized
        .get_mut("scan")
        .and_then(Value::as_object_mut)
        .expect("report has a scan object");
    scan.insert(
        "target".to_string(),
        Value::String(NORMALIZED_TARGET_LABEL.to_string()),
    );
    normalized
}

fn normalized_text(report: &Value) -> String {
    serde_json::to_string_pretty(&normalize(report)).expect("normalized report serializes")
}

/// Every JSON-pointer path (leaf, or the point of a structural mismatch)
/// at which `actual` differs from `baseline`, skipping any path declared
/// in `declared_deltas` (a path is skipped if it equals a declared entry
/// or is nested under one).
fn diff_paths(baseline: &Value, actual: &Value, declared_deltas: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    walk_diff(baseline, actual, String::new(), declared_deltas, &mut out);
    out
}

fn is_declared(path: &str, declared_deltas: &[&str]) -> bool {
    declared_deltas
        .iter()
        .any(|declared| path == *declared || path.starts_with(&format!("{declared}/")))
}

fn walk_diff(
    baseline: &Value,
    actual: &Value,
    path: String,
    declared_deltas: &[&str],
    out: &mut Vec<String>,
) {
    if is_declared(&path, declared_deltas) {
        return;
    }
    match (baseline, actual) {
        (Value::Object(b), Value::Object(a)) => {
            let mut keys: Vec<&String> = b.keys().chain(a.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child_path = format!("{path}/{key}");
                match (b.get(key), a.get(key)) {
                    (Some(bv), Some(av)) => walk_diff(bv, av, child_path, declared_deltas, out),
                    _ => out.push(child_path),
                }
            }
        }
        (Value::Array(b), Value::Array(a)) => {
            if b.len() != a.len() {
                out.push(path);
                return;
            }
            for (index, (bv, av)) in b.iter().zip(a.iter()).enumerate() {
                walk_diff(bv, av, format!("{path}/{index}"), declared_deltas, out);
            }
        }
        _ => {
            if baseline != actual {
                out.push(path);
            }
        }
    }
}

fn read_baseline(relative: &str) -> Value {
    let text = fs::read_to_string(manifest_path(relative))
        .unwrap_or_else(|error| panic!("reading committed baseline {relative}: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parsing committed baseline {relative}: {error}"))
}

#[test]
fn test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline() {
    let (_fixture_dir, _output_dir, report) = scan_corpus(CLEAN_CORPUS_SOURCES);
    let actual_text = normalized_text(&report);
    let baseline_text = fs::read_to_string(manifest_path(CLEAN_BASELINE_PATH))
        .expect("committed clean baseline exists");
    assert_eq!(
        actual_text, baseline_text,
        "clean corpus report.json diverged from the committed pre-IR baseline"
    );
}

#[test]
fn test_malformed_corpus_report_matches_its_baseline_with_declared_deltas_only() {
    let (_fixture_dir, _output_dir, report) = scan_corpus(MALFORMED_CORPUS_SOURCES);
    let actual = normalize(&report);
    let baseline = read_baseline(MALFORMED_BASELINE_PATH);
    let diffs = diff_paths(&baseline, &actual, DECLARED_DELTAS);
    assert!(
        diffs.is_empty(),
        "malformed corpus report.json carries undeclared deltas: {diffs:?}"
    );
}

#[test]
fn test_comparator_detects_a_perturbed_baseline() {
    let baseline = read_baseline(CLEAN_BASELINE_PATH);
    let mut perturbed = baseline.clone();
    let top25 = perturbed
        .get_mut("top25")
        .and_then(Value::as_array_mut)
        .expect("baseline has a top25 array");
    assert!(
        !top25.is_empty(),
        "baseline's top25 must be non-empty to perturb it"
    );
    let cc = top25[0]["cc"]
        .as_u64()
        .expect("top25 entry has a numeric cc");
    top25[0]["cc"] = Value::from(cc + 1);

    let diffs = diff_paths(&baseline, &perturbed, DECLARED_DELTAS);
    assert!(
        !diffs.is_empty(),
        "perturbing one cc value must be detected"
    );
    assert!(
        diffs.iter().any(|path| path.contains("top25")),
        "diff must name top25, got {diffs:?}"
    );
}

#[test]
fn test_normalization_replaces_only_the_scan_target() {
    let (_fixture_dir, _output_dir, report) = scan_corpus(CLEAN_CORPUS_SOURCES);
    let normalized = normalize(&report);
    let diffs = diff_paths(&report, &normalized, &[]);
    assert_eq!(
        diffs,
        vec!["/scan/target".to_string()],
        "normalization must touch exactly scan.target: {diffs:?}"
    );
    assert_eq!(
        normalized["scan"]["target"],
        Value::String(NORMALIZED_TARGET_LABEL.to_string())
    );
}

#[test]
fn test_corpus_copy_is_outside_any_git_work_tree() {
    let (_fixture_dir, _output_dir, report) = scan_corpus(CLEAN_CORPUS_SOURCES);
    assert_eq!(report["scan"]["revision"]["sha"], Value::Null);
    assert_eq!(report["scan"]["revision"]["dirty"], Value::Null);
    assert_eq!(
        report["scan"]["revision"]["unavailable_reason"],
        Value::String("not_a_git_repository".to_string())
    );
}

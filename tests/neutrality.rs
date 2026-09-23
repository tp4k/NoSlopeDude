//! WS-2: the measurement-neutrality gate `nsd-plan-final.md` M0b item 8
//! requires -- scans a fixed, copied fixture corpus with fixed settings and
//! asserts the rendered `report.json` is byte-identical to a committed
//! pre-IR baseline, captured now while the tree is still pre-IR.
//!
//! This suite never shells out to git and needs no worktree, no network and
//! no private archive for its always-on legs:
//! `scripts/neutrality_gate.sh` (the operator-run re-capture/comparison
//! tool) is the mechanism that reaches into git history, kept deliberately
//! separate. The one leg that does need a private fixture --
//! `test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report`
//! -- is env-var gated (`NSD_ARCHIVED_REPORT`) and reports pending, not
//! passing, when that fixture is absent.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use nsd::discover::DiscoverResult;
use nsd::model::{LanguageFamily, ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;

/// Paths relative to `tests/fixtures/`. Decision 1: the malformed corpus is
/// exactly these three in-repo damaged fixtures -- the local stand-ins for
/// the private `js-ts-fixture-01/-02` deltas; every other file under
/// `tests/fixtures/` is the clean corpus.
const MALFORMED_CORPUS_SOURCES: &[&str] = &[
    "metrics/broken/Broken.ts",
    "rules/broken/Broken.java",
    "report/src/Broken.java",
];

const FIXTURES_ROOT: &str = "tests/fixtures";

const CLEAN_BASELINE_PATH: &str = "tests/golden/neutrality/clean.report.json";
const MALFORMED_BASELINE_PATH: &str = "tests/golden/neutrality/malformed.report.json";

/// The label `normalize`/`normalize_raw_text` substitute for the corpus's
/// volatile tempdir path -- the one field that can never be made constant
/// across invocations, because each invocation (including the one that
/// captured the committed baseline) copies the corpus into a fresh tempdir.
const NORMALIZED_TARGET_LABEL: &str = "<neutrality-corpus>";

/// Declared measurement deltas the malformed corpus is permitted to carry
/// against its baseline -- JSON pointers such as `/skipped_files/0/reason`.
/// Empty at this stream: WS-2 captures the pre-IR baseline with no
/// analyzer retargeted yet, so there is nothing to declare. WS-6 (salvage,
/// the `SkipReason` split) populates this list when it lands the deltas
/// M0b item 8 explicitly permits.
const DECLARED_DELTAS: &[&str] = &[];

/// Set (non-empty) to make the two corpus tests below overwrite their
/// baseline files with a freshly captured, normalized report instead of
/// comparing against them. This is the one code path that writes
/// `tests/golden/neutrality/`; a plain `cargo test` never sets it.
/// `scripts/neutrality_gate.sh --capture` is the operator entry point.
const NEUTRALITY_CAPTURE_ENV_VAR: &str = "NSD_NEUTRALITY_CAPTURE";

/// The env var carrying the private archived `java-fixture-01` report's
/// path at invocation time only; it is never written into a repository
/// file (`AGENTS.md`, *Fixture privacy*).
const ARCHIVED_REPORT_ENV_VAR: &str = "NSD_ARCHIVED_REPORT";

/// Opt-in env var that promotes the pending arm of the archive-backed leg
/// from a silent `ok` to a panic, mirroring
/// `tests/golden_digest.rs::REQUIRE_ARCHIVE_VERIFIED_ENV_VAR`.
const REQUIRE_ARCHIVE_VERIFIED_ENV_VAR: &str = "NSD_REQUIRE_ARCHIVE_VERIFIED";

fn manifest_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Every file under `tests/fixtures/` (paths relative to it), sorted for
/// determinism -- decision 1's "copy of `tests/fixtures/`" corpus, before
/// it is split into clean and malformed.
fn all_fixture_relative_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_fixtures(&manifest_path(FIXTURES_ROOT), Path::new(""), &mut out);
    out.sort();
    out
}

fn walk_fixtures(dir: &Path, relative: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("reading fixtures directory {dir:?}: {error}"));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("reading a fixtures dir entry: {error}"));
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("reading a fixtures dir entry's file type: {error}"));
        let child_relative = relative.join(entry.file_name());
        if file_type.is_dir() {
            walk_fixtures(&dir.join(entry.file_name()), &child_relative, out);
        } else if file_type.is_file() {
            out.push(child_relative);
        }
    }
}

fn malformed_corpus_sources() -> Vec<PathBuf> {
    MALFORMED_CORPUS_SOURCES.iter().map(PathBuf::from).collect()
}

/// All of `tests/fixtures/` minus the three malformed sources (decision 1).
fn clean_corpus_sources() -> Vec<PathBuf> {
    let malformed = malformed_corpus_sources();
    all_fixture_relative_paths()
        .into_iter()
        .filter(|path| !malformed.contains(path))
        .collect()
}

/// Copies `sources` (paths relative to `tests/fixtures/`) into `dest_root`,
/// mirroring each source's own relative path rather than flattening it --
/// preserving directory structure is what lets the D16 exclusion globs
/// (`node_modules/`, `__tests__/`, ...) actually fire on the copy, and lets
/// same-basename fixtures in different directories (`Sample.java`,
/// `sample.js`, `mod.mjs`, `HighComplexity.java`, `Decisions.java`) coexist
/// instead of silently overwriting one another.
fn copy_corpus(sources: &[PathBuf], dest_root: &Path) {
    let mut written: HashSet<PathBuf> = HashSet::new();
    for relative in sources {
        let dest = dest_root.join(relative);
        if !written.insert(dest.clone()) {
            panic!(
                "corpus copy collision: {relative:?} would overwrite an earlier copy at {dest:?}"
            );
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).expect("create corpus destination directory");
        }
        let from = manifest_path(FIXTURES_ROOT).join(relative);
        fs::copy(&from, &dest)
            .unwrap_or_else(|error| panic!("copying corpus source {from:?} to {dest:?}: {error}"));
    }
}

/// Everything one scan of a freshly copied corpus produces that this suite
/// needs: the rendered `report.json` text, the tempdir path substituted for
/// `scan.target`, and the discovery result the pipeline itself computed (so
/// the corpus-presence assertion below re-reads the pipeline's own
/// discovery, rather than repeating a second, possibly-diverging walk).
struct ScanCorpusOutput {
    _fixture_dir: tempfile::TempDir,
    _output_dir: tempfile::TempDir,
    json_text: String,
    target_input: String,
    discover: DiscoverResult,
}

/// Copies `sources` into a fresh tempdir and scans it with fixed settings
/// -- the same tempdir-fixture / tempdir-output / explicit-`ScanSettings`
/// shape `tests/e2e_local.rs::run_scan` uses. `include_tests: true`
/// (decision 3): `src/discover.rs`'s default test globs exclude
/// `__tests__/`, where most of `tests/fixtures/clones` and
/// `tests/fixtures/metrics` lives, so without it the corpus would cover
/// almost nothing.
fn scan_corpus(sources: &[PathBuf]) -> ScanCorpusOutput {
    let fixture_dir = tempfile::tempdir().expect("fixture tempdir");
    copy_corpus(sources, fixture_dir.path());
    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: true,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    let target_input = fixture_dir
        .path()
        .to_str()
        .expect("fixture path is valid UTF-8")
        .to_string();
    let pipeline_output = pipeline::run(&target_input, settings)
        .unwrap_or_else(|error| panic!("pipeline run should succeed: {error}"));
    let json_text =
        fs::read_to_string(output_dir.path().join("report.json")).expect("report.json exists");
    ScanCorpusOutput {
        _fixture_dir: fixture_dir,
        _output_dir: output_dir,
        json_text,
        target_input,
        discover: pipeline_output.discover,
    }
}

fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("parsing report JSON: {error}"))
}

fn has_known_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| LanguageFamily::from_extension(extension).is_some())
}

/// Every corpus source with one of the seven scanned extensions must be
/// either discovered or (explicitly) skipped -- proving the mirrored-path
/// copy above lost nothing to a basename collision or a path error, and
/// that the D16 exclusion paths (`node_modules/`, `.gitignore`, generated
/// code) actually fire rather than being bypassed. A source with no known
/// extension (e.g. a fixture's own `.gitignore`) is never classified by
/// discovery at all, so it is not asserted on here.
fn assert_every_source_is_discovered_or_skipped(sources: &[PathBuf], discover: &DiscoverResult) {
    let mut accounted: HashSet<&PathBuf> = discover
        .discovered
        .iter()
        .map(|file| &file.relative_path)
        .collect();
    accounted.extend(discover.skipped.iter().map(|file| &file.relative_path));
    for source in sources {
        if has_known_extension(source) {
            assert!(
                accounted.contains(source),
                "corpus source {source:?} was neither discovered nor skipped -- \
                 lost by the corpus copy"
            );
        }
    }
}

fn capture_requested() -> bool {
    std::env::var_os(NEUTRALITY_CAPTURE_ENV_VAR).is_some_and(|value| !value.is_empty())
}

/// Replaces exactly one occurrence of `target_input` (the corpus's own
/// tempdir path) with the fixed label, in the raw text `render_json` wrote
/// -- not a `serde_json::Value` re-serialization, which would reorder keys
/// (`Value` is a `BTreeMap`) and hide a real serialization-shape change.
fn normalize_raw_text(json_text: &str, target_input: &str) -> String {
    let occurrences = json_text.matches(target_input).count();
    assert_eq!(
        occurrences, 1,
        "expected exactly one occurrence of the corpus tempdir path {target_input:?} \
         in the rendered report, found {occurrences}"
    );
    json_text.replacen(target_input, NORMALIZED_TARGET_LABEL, 1)
}

/// Replaces `scan.target` with the fixed label and nothing else, on the
/// parsed `Value` -- used by the malformed-corpus JSON-pointer diff and by
/// `test_normalization_replaces_only_the_scan_target`, which asserts this
/// touches exactly one field.
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
                    _ if is_declared(&child_path, declared_deltas) => {}
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
    parse_json(&text)
}

#[test]
fn test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline() {
    let scanned = scan_corpus(&clean_corpus_sources());
    assert_every_source_is_discovered_or_skipped(&clean_corpus_sources(), &scanned.discover);
    let actual_text = normalize_raw_text(&scanned.json_text, &scanned.target_input);

    let baseline_path = manifest_path(CLEAN_BASELINE_PATH);
    if capture_requested() {
        fs::write(&baseline_path, &actual_text).expect("write clean neutrality baseline");
        return;
    }
    let baseline_text =
        fs::read_to_string(&baseline_path).expect("committed clean baseline exists");
    assert_eq!(
        actual_text, baseline_text,
        "clean corpus report.json diverged from the committed pre-IR baseline"
    );
}

#[test]
fn test_malformed_corpus_report_matches_its_baseline_with_declared_deltas_only() {
    let scanned = scan_corpus(&malformed_corpus_sources());
    assert_every_source_is_discovered_or_skipped(&malformed_corpus_sources(), &scanned.discover);
    let actual_text = normalize_raw_text(&scanned.json_text, &scanned.target_input);

    let baseline_path = manifest_path(MALFORMED_BASELINE_PATH);
    if capture_requested() {
        fs::write(&baseline_path, &actual_text).expect("write malformed neutrality baseline");
        return;
    }
    let actual = normalize(&parse_json(&scanned.json_text));
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

/// Direct unit test on `diff_paths`, independent of any scan or baseline
/// file: covers the three shapes the malformed-corpus test alone cannot
/// exercise (its own `DECLARED_DELTAS` is empty by design at this stream).
#[test]
fn test_diff_paths_suppresses_only_declared_deltas() {
    const DECLARED: &[&str] = &["/scores/overall/erosion", "/skipped_files/0/detail"];

    let baseline = json!({
        "scores": {
            "overall": {"erosion": 0.1},
            "java": {"erosion": 0.2}
        },
        "skipped_files": [
            {"relative_path": "a.java", "reason": "test"}
        ]
    });

    // A changed leaf directly under a declared pointer is suppressed.
    let mut changed_leaf = baseline.clone();
    changed_leaf["scores"]["overall"]["erosion"] = json!(0.9);
    assert_eq!(
        diff_paths(&baseline, &changed_leaf, DECLARED),
        Vec::<String>::new(),
        "a changed leaf under a declared pointer must be suppressed"
    );

    // A key present on only one side (here: added), nested under a
    // declared pointer, is suppressed -- this is the HIGH-severity fix:
    // the object arm's one-sided fallthrough must consult declared_deltas,
    // not only the leaf-compare path a recursive call's entry check covers.
    let mut key_added = baseline.clone();
    key_added["skipped_files"][0]["detail"] = json!("parse error");
    assert_eq!(
        diff_paths(&baseline, &key_added, DECLARED),
        Vec::<String>::new(),
        "a key added under a declared pointer must be suppressed"
    );

    // A changed leaf outside any declared pointer is still reported.
    let mut changed_outside = baseline.clone();
    changed_outside["scores"]["java"]["erosion"] = json!(0.9);
    assert_eq!(
        diff_paths(&baseline, &changed_outside, DECLARED),
        vec!["/scores/java/erosion".to_string()],
        "a changed leaf outside any declared pointer must still be reported"
    );
}

#[test]
fn test_normalization_replaces_only_the_scan_target() {
    let scanned = scan_corpus(&clean_corpus_sources());
    let report = parse_json(&scanned.json_text);
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
    let scanned = scan_corpus(&clean_corpus_sources());
    let report = parse_json(&scanned.json_text);
    assert_eq!(report["scan"]["revision"]["sha"], Value::Null);
    assert_eq!(report["scan"]["revision"]["dirty"], Value::Null);
    assert_eq!(
        report["scan"]["revision"]["unavailable_reason"],
        Value::String("not_a_git_repository".to_string())
    );
}

/// Where the archive is, or an explicit statement that this leg is pending
/// because the private fixture was not supplied -- mirroring
/// `tests/golden_digest.rs::ArchiveGate` so a missing private fixture can
/// never silently read as a validated pass (`AGENTS.md` -> *Verification*).
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

fn pending_notice() -> String {
    format!(
        "PENDING: {ARCHIVED_REPORT_ENV_VAR} is unset -- the java-fixture-01 strict \
         neutrality leg is pending, not passing, this run"
    )
}

fn verification_required() -> bool {
    classify_verification_requirement(std::env::var_os(REQUIRE_ARCHIVE_VERIFIED_ENV_VAR))
}

/// Pure classification of a possibly-absent `NSD_REQUIRE_ARCHIVE_VERIFIED`
/// value, split out the same way `classify_archive_gate` is: so `None` and
/// `Some("")` (unset, and set-but-empty) can each be asserted directly as
/// "not required", rather than only observed through the process's own
/// environment.
fn classify_verification_requirement(raw: Option<OsString>) -> bool {
    matches!(raw, Some(value) if !value.is_empty())
}

fn required_but_pending_message() -> String {
    format!(
        "{REQUIRE_ARCHIVE_VERIFIED_ENV_VAR} demands a verified run, but \
         {ARCHIVED_REPORT_ENV_VAR} is unset -- supply the archive or unset \
         {REQUIRE_ARCHIVE_VERIFIED_ENV_VAR}"
    )
}

/// Item 8's strict leg: `java-fixture-01` has no parse failures, so neither
/// salvage nor the `SkipReason` split can mask an IR defect there. Re-scans
/// the archive's own recorded target with its own recorded settings and
/// asserts the freshly rendered `report.json` is byte-identical to the
/// archived one. The archive is resolved only from `NSD_ARCHIVED_REPORT` at
/// invocation time and never committed (`AGENTS.md`, *Fixture privacy*).
#[test]
fn test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report() {
    let path = match archive_gate() {
        ArchiveGate::Resolved(path) => path,
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
            if verification_required() {
                panic!("{}", required_but_pending_message());
            }
            return;
        }
    };

    let archived_text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading the archived report: {error}"));
    let archived = parse_json(&archived_text);
    let scan = archived
        .get("scan")
        .and_then(Value::as_object)
        .expect("archived report has a scan object");
    let target = scan
        .get("target")
        .and_then(Value::as_str)
        .expect("scan.target is a string")
        .to_string();
    let include_tests = scan
        .get("include_tests")
        .and_then(Value::as_bool)
        .expect("scan.include_tests is a bool");
    let exclude: Vec<String> = scan
        .get("exclude")
        .and_then(Value::as_array)
        .expect("scan.exclude is an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("scan.exclude entry is a string")
                .to_string()
        })
        .collect();
    let min_clone_lines = scan
        .get("min_clone_lines")
        .and_then(Value::as_u64)
        .expect("scan.min_clone_lines is a number") as u32;

    let output_dir = tempfile::tempdir().expect("output tempdir");
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests,
        exclude,
        min_clone_lines,
    };
    pipeline::run(&target, settings)
        .unwrap_or_else(|error| panic!("scanning the archive's recorded target: {error}"));
    let actual_text = fs::read_to_string(output_dir.path().join("report.json"))
        .expect("freshly rendered report.json exists");

    assert!(
        actual_text == archived_text,
        "the IR build must render java-fixture-01 byte-identical to the archived report; \
         lengths {} vs {}, first differing byte at {:?}; contents withheld \
         (AGENTS.md, Fixture privacy)",
        actual_text.len(),
        archived_text.len(),
        actual_text
            .bytes()
            .zip(archived_text.bytes())
            .position(|(a, b)| a != b)
    );
}

/// Proves the pending path is reachable and prints its notice rather than
/// silently substituting a pass, independent of whether this invocation
/// happens to carry the archive-backed leg too -- mirroring
/// `tests/golden_digest.rs::test_gate_is_reported_pending_when_the_archive_is_absent`.
#[test]
fn test_java_fixture_01_strict_leg_is_reported_pending_when_the_archive_is_absent() {
    match archive_gate() {
        ArchiveGate::Pending => println!("{}", pending_notice()),
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
/// under -- mirroring `tests/golden_digest.rs::test_classify_archive_gate`.
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

/// `classify_verification_requirement` is the pure decision
/// `verification_required` delegates to; tested directly for the same
/// reason `classify_archive_gate` is -- the unset and set-but-empty cases
/// must read as "not required", not just happen to -- mirroring
/// `tests/golden_digest.rs::test_classify_verification_requirement_needs_a_non_empty_value`.
#[test]
fn test_classify_verification_requirement_needs_a_non_empty_value() {
    assert!(!classify_verification_requirement(None));
    assert!(!classify_verification_requirement(Some(OsString::new())));
    assert!(classify_verification_requirement(Some(OsString::from("1"))));
}

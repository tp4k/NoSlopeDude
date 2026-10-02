//! WS-2: the measurement-neutrality gate `nsd-plan-final.md` M0b item 8
//! requires -- scans a fixed, copied fixture corpus with fixed settings and
//! asserts the rendered `report.json` is byte-identical to a committed
//! pre-IR baseline, captured now while the tree is still pre-IR.
//!
//! This suite never shells out to git, needs no worktree, no network and no
//! private archive: `scripts/neutrality_gate.sh` (the operator-run
//! re-capture/comparison tool) is the mechanism that reaches into git
//! history, kept deliberately separate. Through M0b this suite also carried
//! a private-archive-gated strict byte-identity leg against
//! `java-fixture-01`; M0c-10 retired it (see `docs/ir-neutrality.md`,
//! *The `java-fixture-01` strict leg*) because it asserted an invariant
//! about holding the Java parser fixed, which that workstream's grammar
//! swap deliberately breaks. The `java-fixture-01` corpus's separate lossy
//! digest comparison (`tests/golden_digest.rs`) is unaffected.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use nsd::discover::DiscoverResult;
use nsd::model::{LanguageFamily, ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline;

/// Paths relative to `tests/fixtures/`. Decision 1: the malformed corpus is
/// exactly these in-repo damaged fixtures -- the local stand-ins for the
/// private `js-ts-fixture-01/-02` deltas; every other file under
/// `tests/fixtures/` is the clean corpus. Unchanged from the pre-WS-6 list:
/// this is also the exact corpus `test_malformed_corpus_report_matches_its_
/// baseline_with_declared_deltas_only` scans as one unit against
/// `malformed.report.json`, so growing it (even with another already-
/// damaged fixture) grows that test's own scanned corpus past what its
/// baseline was captured from -- a corpus-*size* change (a `/top25` length
/// mismatch, `scores.*.verbosity.scanned_lines` moved by files the baseline
/// never saw), which `DECLARED_DELTAS` has no way to express as a per-field
/// delta. `tests/fixtures/salvage/`'s own two fixtures deliberately do NOT
/// join this list -- per Decision 20 they join the *clean* corpus instead
/// (`clean_corpus_sources()` below), with `clean.report.json` re-captured
/// to account for them, exactly as WS-3 round 1's `tests/fixtures/parity/`
/// did.
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
/// WS-6 (salvage, the `SkipReason` split) populates this list with exactly
/// the fields item 8's declared exception authorizes moving on
/// `MALFORMED_CORPUS_SOURCES`'s three fixtures:
///
/// - `/skipped_files` -- a `SyntaxError` file is no longer a whole-file
///   skip, but `src/report/mod.rs::build_skipped_files` still lists it so
///   the file behind `incomplete` is named: all three fixtures' entries
///   stay, and only each `detail` moves from `null` to
///   `salvaged; first error at line N`.
/// - `/scores/overall/verbosity/scanned_lines` and
///   `/scores/java/verbosity/scanned_lines` -- `rules/broken/Broken.java`
///   and `report/src/Broken.java` each have one callable and it intersects
///   the damage, so it stays unmeasured (fail-closed), but D12's per-file
///   scanned-line count is unconditioned on callable boundaries and still
///   walks each file's surviving class-wrapper lines around the pruned
///   callable -- previously 0 (the whole file was dropped), now a few per
///   file. `js_ts`'s own `metrics/broken/Broken.ts` has no such surviving
///   wrapper content, so `/scores/js_ts/verbosity/scanned_lines` needs no
///   entry: it stays `0` on both sides. `flagged_lines` and every `ratio`
///   also stay identical (`0`, since `0/0` and `0/scanned_lines` are both
///   `0.0`) and need no entry either -- none of the three fixtures'
///   surviving wrapper lines trip any of the six rules.
const DECLARED_DELTAS: &[&str] = &[
    "/skipped_files",
    "/scores/overall/verbosity/scanned_lines",
    "/scores/java/verbosity/scanned_lines",
];

/// Set (non-empty) to make the two corpus tests below overwrite their
/// baseline files with a freshly captured, normalized report instead of
/// comparing against them. This is the one code path that writes
/// `tests/golden/neutrality/`; a plain `cargo test` never sets it.
/// `scripts/neutrality_gate.sh --capture` is the operator entry point.
const NEUTRALITY_CAPTURE_ENV_VAR: &str = "NSD_NEUTRALITY_CAPTURE";

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
/// `tests/fixtures/salvage/`'s two fixtures (`Mixed.java`, `Mixed.ts`) join
/// this corpus like any other fixture -- see `MALFORMED_CORPUS_SOURCES`'s
/// own doc comment for why they don't belong there instead.
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

#[derive(Clone, Copy)]
enum Corpus {
    Clean,
    Malformed,
}

fn capture_selected(
    _corpus: Corpus,
    _read_var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> bool {
    unimplemented!("per-corpus capture selection")
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

    // `is_declared` suppresses every change under `/skipped_files`, but the
    // declared delta's intent is only the documented `detail` move on the
    // three salvaged rows, not "any change to this array is fine". Pin the
    // whole array so a stray, missing or re-reasoned row cannot hide behind
    // the declared delta.
    assert_eq!(
        actual["skipped_files"],
        json!([
            {
                "relative_path": "metrics/broken/Broken.ts",
                "reason": "parse_syntax_error",
                "detail": "salvaged; first error at line 1"
            },
            {
                "relative_path": "report/src/Broken.java",
                "reason": "parse_syntax_error",
                "detail": "salvaged; first error at line 2"
            },
            {
                "relative_path": "rules/broken/Broken.java",
                "reason": "parse_syntax_error",
                "detail": "salvaged; first error at line 5"
            }
        ]),
        "every MALFORMED_CORPUS_SOURCES entry is listed as a salvaged skip"
    );
}

/// Every fixture under `tests/fixtures/` must land in exactly one of the two
/// corpora -- a fixture excluded from both (as `salvage/Mixed.java` and
/// `salvage/Mixed.ts` were, via `CLEAN_CORPUS_ONLY_EXCLUSIONS`) is covered by
/// neither corpus test, silently. If a future fixture needs excluding from
/// the clean corpus, `MALFORMED_CORPUS_SOURCES` is the only sanctioned way
/// to do that -- growing it, or capturing a fresh malformed baseline -- not
/// a third exclusion list.
#[test]
fn test_every_fixture_is_in_exactly_one_corpus() {
    let clean = clean_corpus_sources();
    let malformed = malformed_corpus_sources();
    let all = all_fixture_relative_paths();
    assert_eq!(
        clean.len() + malformed.len(),
        all.len(),
        "clean_corpus_sources() ({}) + malformed_corpus_sources() ({}) must equal \
         all_fixture_relative_paths() ({}) -- a fixture fell outside both corpora",
        clean.len(),
        malformed.len(),
        all.len()
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

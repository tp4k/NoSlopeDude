//! WS-5: `report.json` — one assertion per bullet-5 item, the scan-settings
//! round trip (D6/D17), the D18 incomplete marker, and the bracket-exact
//! source spans acceptance criterion.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use git2::Repository;

use nsd::git::snapshot::WorktreeSnapshot;
use nsd::git::snapshot_id::SnapshotId;
use nsd::model::{RemoteTarget, Revision, ScanSettings, Target, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline::{self, PipelineOutput};
use nsd::profile::{fingerprint, MeasurementProfileInputs};
use nsd::report::{self, ReportInput};

/// A real, on-disk git worktree with a committed `HEAD` (system `git` binary,
/// as `target::local_git_revision` reads it via `rev-parse`), used by
/// `test_local_git_scan_reports_null_dirty_even_after_an_edit` below. Kept local to
/// this file rather than `tests/common/mod.rs`: each `tests/*.rs` file is
/// its own crate for `cargo clippy`'s dead-code lint, and no other test file
/// calls this helper, so sharing it there would leave it (and `run_git`)
/// flagged as unused dead code in every other suite under `-D warnings`.
fn init_git_worktree(dir: &Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "fixture@example.invalid"]);
    run_git(dir, &["config", "user.name", "nsd test fixture"]);
}

/// Stages every file under `dir` and commits it to `HEAD`.
fn git_commit_all(dir: &Path, message: &str) {
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-m", message]);
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/report")
}

/// Runs the full pipeline against `root`, writing the two reports into a
/// fresh temp directory that is kept alive for the caller's lifetime.
fn run_scan(
    root: &Path,
    configure: impl FnOnce(&mut ScanSettings),
) -> (tempfile::TempDir, PipelineOutput) {
    let output_dir = tempfile::tempdir().expect("tempdir");
    let mut settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: true,
        exclude: Vec::new(),
        min_clone_lines: DEFAULT_MIN_CLONE_LINES,
    };
    configure(&mut settings);
    let target_input = root
        .to_str()
        .expect("fixture path is valid UTF-8")
        .to_string();
    let output = pipeline::run(&target_input, settings).expect("pipeline run should succeed");
    (output_dir, output)
}

#[test]
fn test_json_report_contains_every_required_section() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let json_text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");

    // Overall and per-language-family scores.
    for family in ["overall", "java", "js_ts"] {
        let scores = &value["scores"][family];
        assert!(scores["erosion"].is_number(), "{family} erosion missing");
        assert!(
            scores["verbosity"]["ratio"].is_number(),
            "{family} verbosity ratio missing"
        );
    }

    // Per-family verbosity is the rules stage's own published score, not an
    // interchangeable slot: value-level equality against
    // `output.rules.verbosity`, plus java != js_ts so a swap is detectable.
    for (family, published) in [
        ("overall", &output.rules.verbosity.overall),
        ("java", &output.rules.verbosity.java),
        ("js_ts", &output.rules.verbosity.js_ts),
    ] {
        assert_eq!(
            value["scores"][family]["verbosity"]["flagged_lines"]
                .as_u64()
                .unwrap(),
            published.flagged_lines as u64,
            "{family} verbosity flagged_lines should match the rules stage's own score"
        );
    }
    assert_ne!(
        output.rules.verbosity.java.flagged_lines, output.rules.verbosity.js_ts.flagged_lines,
        "fixture should distinguish java and js_ts verbosity so a slot swap is detectable"
    );

    // Rule IDs and source excerpts.
    let findings = value["findings"].as_array().expect("findings array");
    assert!(!findings.is_empty(), "fixture should trigger rule findings");
    assert_eq!(findings.len(), output.rules.findings.len());
    for (index, finding) in findings.iter().enumerate() {
        assert!(finding["rule_id"].is_string());
        assert!(
            finding["location"].get("excerpt").is_none(),
            "the canonical report carries no source excerpt: {finding:?}"
        );

        // Value-level equality against the rules stage's own published
        // finding at the same index (D20/D22): a family_label swap or a
        // dropped flagged-line does not fool this.
        let expected = &output.rules.findings[index];
        let path = finding["location"]["relative_path"].as_str().unwrap();
        let language = finding["language"].as_str().unwrap();
        assert_eq!(
            language == "java",
            path.ends_with(".java"),
            "a finding's language should match its file extension: {finding:?}"
        );
        assert_eq!(finding["rule_id"].as_str().unwrap(), expected.rule_id);
        let flagged_lines: Vec<usize> = finding["flagged_lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(flagged_lines, expected.flagged_lines);
    }

    // Duplicate locations.
    let duplicates = value["duplicates"].as_array().expect("duplicates array");
    assert!(
        !duplicates.is_empty(),
        "fixture should trigger a clone group"
    );
    assert_eq!(duplicates.len(), output.clones.groups.len());
    for (index, group) in duplicates.iter().enumerate() {
        let locations = group["locations"].as_array().expect("locations array");
        assert!(
            locations.len() >= 2,
            "a clone group has at least two occurrences"
        );
        for location in locations {
            assert!(location["relative_path"].is_string());
        }
        assert_eq!(
            group["redundant_lines"].as_u64().unwrap() as usize,
            output.clones.groups[index].redundant_lines,
            "redundant_lines should match the clones stage's own published group"
        );
    }

    // Top-25 functions.
    let top25 = value["top25"].as_array().expect("top25 array");
    assert!(
        !top25.is_empty(),
        "fixture should have at least one callable"
    );
    assert_eq!(top25.len(), output.metrics.top25.len());
    for (index, callable) in top25.iter().enumerate() {
        assert!(callable["name"].is_string());
        assert!(callable["cc"].is_number());

        // Value-level equality against the metrics stage's own published
        // top-25 row at the same index (D8/D13).
        let expected = &output.metrics.top25[index];
        let path = callable["location"]["relative_path"].as_str().unwrap();
        let language = callable["language"].as_str().unwrap();
        assert_eq!(
            language == "java",
            path.ends_with(".java"),
            "a top-25 row's language should match its file extension: {callable:?}"
        );
        assert_eq!(callable["cc"].as_u64().unwrap() as u32, expected.cc);
        assert_eq!(callable["sloc"].as_u64().unwrap() as usize, expected.sloc);
        assert!((callable["mass"].as_f64().unwrap() - expected.mass).abs() < 1e-9);
    }

    // Scan settings.
    assert!(
        value["scan"]["target"].is_null(),
        "a local target is published as null, never as a path"
    );
    assert!(value["scan"]["include_tests"].is_boolean());
    assert!(value["scan"]["exclude"].is_array());
    assert!(value["scan"]["min_clone_lines"].is_number());
    assert!(value["scan"]["revision"].is_object());
    assert!(
        value["scan"]["revision"]["dirty"].is_null(),
        "a scan inside a local git work tree publishes dirty as null"
    );

    // Skipped files: `Broken.java` salvage-parses, and is listed with a
    // `salvaged` detail naming its first error line.
    let skipped = value["skipped_files"]
        .as_array()
        .expect("skipped_files array");
    assert!(
        skipped
            .iter()
            .any(|file| file["reason"] == "parse_syntax_error"
                && file["detail"] == "salvaged; first error at line 2"),
        "the syntax-error file is listed as a salvaged skip: {skipped:?}"
    );

    // Assumptions-mandated adaptation label (non-equivalence disclosure).
    assert_eq!(
        value["adaptation"]["cc_rules_doc"].as_str().unwrap(),
        "docs/cc-rules.md"
    );
    assert!(!value["adaptation"]["summary"].as_str().unwrap().is_empty());
}

fn fixture_erosion_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/report_erosion")
}

#[test]
fn test_erosion_is_computed_per_language_family() {
    // The only other fixture (`tests/fixtures/report`) has no callable with
    // `cc > CC_EROSION_THRESHOLD`, so every family's erosion is `0.0` there
    // — a `java_callables`/`js_ts_callables` slot swap (D20) is
    // undetectable on it. This fixture has one Java callable with 11 `if`
    // statements (`cc == 12`) and no JS/TS file at all, so java erosion
    // must be nonzero while js_ts erosion stays exactly zero.
    let (_dir, output) = run_scan(&fixture_erosion_root(), |_| {});
    let json_text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");

    assert!(
        value["scores"]["java"]["erosion"].as_f64().unwrap() > 0.0,
        "the high-complexity Java callable should erode the java score: {value}"
    );
    assert_eq!(
        value["scores"]["js_ts"]["erosion"].as_f64().unwrap(),
        0.0,
        "a fixture with no JS/TS callable should have zero js_ts erosion: {value}"
    );
    assert_eq!(
        value["scores"]["overall"]["erosion"].as_f64().unwrap(),
        output.metrics.erosion,
        "overall erosion is WS-2's own published value"
    );
}

#[test]
fn test_erosion_zero_is_never_rendered_as_negative_zero() {
    // `metrics::erosion` sums an empty filtered slice when no callable
    // exceeds the CC threshold; on this toolchain `Iterator::sum` over an
    // empty `f64` sequence is `-0.0`, and `-0.0 / total_mass` stays `-0.0`.
    // `-0.0 == 0.0` numerically, but a report literally printing
    // "-0.0000" reads as a defect to a human, so the report layer
    // normalizes the sign of an exact zero on display — it does not
    // change any nonzero score.
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let json_text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    assert!(
        !json_text.contains("-0.0"),
        "no score should render as negative zero: {json_text}"
    );
}

#[test]
fn test_scan_settings_round_trip() {
    // A fresh, never-git-initialized directory isolates this assertion from
    // this repo's own ambient git state (this tree is a live git work tree
    // shared by other workstreams' agents, so its `dirty`/`sha` are not
    // this test's to depend on).
    let source_dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        source_dir.path().join("Sample.java"),
        "public class Sample { public void run() {} }\n",
    )
    .expect("write fixture file");

    let (_dir, output) = run_scan(source_dir.path(), |settings| {
        settings.include_tests = false;
        settings.exclude = vec!["**/vendor/**".to_string(), "**/generated/**".to_string()];
        settings.min_clone_lines = 42;
    });

    let scan = &output.report.scan;
    assert!(!scan.include_tests);
    assert_eq!(
        scan.exclude,
        vec!["**/vendor/**".to_string(), "**/generated/**".to_string()]
    );
    assert_eq!(scan.min_clone_lines, 42);
    assert_eq!(scan.target, source_dir.path().to_str().unwrap().to_string());
    assert_eq!(
        scan.revision.unavailable_reason.as_deref(),
        Some("not_a_git_repository")
    );
    assert_eq!(scan.revision.sha, None);
    assert_eq!(scan.revision.dirty, None);

    // A clean, fully-parseable scan is not incomplete (D18): hard-coding
    // `incomplete: true` unconditionally would pass every other test here,
    // since they all scan a fixture with a deliberate parse failure.
    assert!(
        !output.report.incomplete,
        "a scan with no parse failure should not be marked incomplete"
    );
}

#[test]
fn test_local_git_scan_reports_null_dirty_even_after_an_edit() {
    let repo_dir = tempfile::tempdir().expect("tempdir");
    init_git_worktree(repo_dir.path());
    fs::write(repo_dir.path().join("A.java"), "public class A {}\n").expect("write A.java");
    git_commit_all(repo_dir.path(), "initial commit");

    let (_dir, output) = run_scan(repo_dir.path(), |_| {});
    assert_eq!(
        output.report.scan.revision.dirty, None,
        "a local git scan computes no dirty flag: {:?}",
        output.report.scan.revision
    );
    assert!(output.report.scan.revision.sha.is_some());

    fs::write(
        repo_dir.path().join("A.java"),
        "public class A { void x() {} }\n",
    )
    .expect("edit A.java");

    let (_dir2, output2) = run_scan(repo_dir.path(), |_| {});
    assert_eq!(
        output2.report.scan.revision.dirty, None,
        "an edited local git scan still computes no dirty flag: {:?}",
        output2.report.scan.revision
    );
    assert!(output2.report.scan.revision.sha.is_some());
}

#[test]
fn test_incomplete_marker_set_on_parse_failure() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    assert!(
        output.report.incomplete,
        "Broken.java's syntax error should mark the report incomplete"
    );

    // `Broken.java` salvage-parses, and `skipped_files` names it as the
    // file that made the report incomplete (Greptile P2 on PR #1).
    let listed = output
        .report
        .skipped_files
        .iter()
        .find(|file| file.relative_path == Path::new("src/Broken.java"))
        .unwrap_or_else(|| panic!("Broken.java is listed: {:?}", output.report.skipped_files));
    assert_eq!(listed.reason, "parse_syntax_error");
    assert_eq!(
        listed.detail.as_deref(),
        Some("salvaged; first error at line 2"),
        "line 2 opens `broken(` and never closes it"
    );
}

fn read_report(output: &PipelineOutput) -> serde_json::Value {
    let json_text =
        fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists");
    serde_json::from_str(&json_text).expect("valid JSON")
}

/// The families' verbosity objects, `overall` last.
fn verbosity_objects(value: &serde_json::Value) -> [&serde_json::Value; 3] {
    ["java", "js_ts", "overall"].map(|family| &value["scores"][family]["verbosity"])
}

/// M1-8: every family and the overall score carry `unanalyzed_lines` and
/// `complete` beside `scanned_lines`. `tests/fixtures/report/src` without its
/// `Broken.java` is the clean corpus; with it, the Java family is salvaged.
#[test]
fn test_scores_carry_completeness_metadata() {
    let clean_dir = tempfile::tempdir().expect("tempdir");
    for entry in fs::read_dir(fixture_root().join("src")).expect("fixture dir") {
        let entry = entry.expect("dir entry");
        if entry.file_name() == "Broken.java" {
            continue;
        }
        fs::copy(entry.path(), clean_dir.path().join(entry.file_name())).expect("copy fixture");
    }
    let (_clean_out, clean) = run_scan(clean_dir.path(), |_| {});
    let clean_value = read_report(&clean);
    let parse_rows: Vec<&serde_json::Value> = clean_value["skipped_files"]
        .as_array()
        .expect("skipped_files array")
        .iter()
        .filter(|row| {
            row["reason"]
                .as_str()
                .is_some_and(|r| r.starts_with("parse_"))
        })
        .collect();
    assert!(
        parse_rows.is_empty(),
        "the clean copy must parse: {parse_rows:?}"
    );
    for verbosity in verbosity_objects(&clean_value) {
        assert_eq!(verbosity["unanalyzed_lines"], 0, "{verbosity}");
        assert_eq!(verbosity["complete"], true, "{verbosity}");
    }

    let (_salvaged_out, salvaged) = run_scan(&fixture_root(), |_| {});
    let value = read_report(&salvaged);
    let [java, js_ts, overall] = verbosity_objects(&value);
    let java_unanalyzed = java["unanalyzed_lines"].as_u64().expect("java count");
    assert!(java_unanalyzed > 0, "Broken.java's pruned lines: {java}");
    assert_eq!(java["complete"], false, "{java}");
    assert_eq!(js_ts["unanalyzed_lines"], 0, "{js_ts}");
    assert_eq!(
        js_ts["complete"], true,
        "a Java damage must not taint js_ts: {js_ts}"
    );
    assert_eq!(overall["unanalyzed_lines"], java_unanalyzed, "{overall}");
    assert_eq!(overall["complete"], false, "{overall}");
}

/// M1-8: a file lost in parsing entirely (no IR, so no line count) still
/// makes its own family `complete: false`, and only its own.
#[test]
fn test_unparsed_file_makes_its_family_incomplete() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("Good.java"),
        "class Good {\n    int x = 1;\n}\n",
    )
    .expect("write Good.java");
    fs::write(dir.path().join("good.js"), "const x = 1;\n").expect("write good.js");
    fs::write(dir.path().join("Bad.java"), [0xff_u8, 0xfe, 0xfd]).expect("write Bad.java");

    let (_out, output) = run_scan(dir.path(), |_| {});
    let value = read_report(&output);
    let bad = value["skipped_files"]
        .as_array()
        .expect("skipped_files array")
        .iter()
        .find(|row| row["relative_path"] == "Bad.java")
        .unwrap_or_else(|| panic!("Bad.java is listed: {}", value["skipped_files"]));
    assert_eq!(bad["reason"], "parse_unreadable");

    let [java, js_ts, overall] = verbosity_objects(&value);
    assert_eq!(java["unanalyzed_lines"], 0, "no count is known: {java}");
    assert_eq!(java["complete"], false, "{java}");
    assert_eq!(js_ts["complete"], true, "{js_ts}");
    assert_eq!(overall["complete"], false, "{overall}");

    let js_dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        js_dir.path().join("Good.java"),
        "class Good {\n    int x = 1;\n}\n",
    )
    .expect("write Good.java");
    fs::write(js_dir.path().join("good.js"), "const x = 1;\n").expect("write good.js");
    fs::write(js_dir.path().join("bad.js"), [0xff_u8, 0xfe, 0xfd]).expect("write bad.js");

    let (_out, output) = run_scan(js_dir.path(), |_| {});
    let value = read_report(&output);
    let bad = value["skipped_files"]
        .as_array()
        .expect("skipped_files array")
        .iter()
        .find(|row| row["relative_path"] == "bad.js")
        .unwrap_or_else(|| panic!("bad.js is listed: {}", value["skipped_files"]));
    assert_eq!(bad["reason"], "parse_unreadable");

    let [java, js_ts, _overall] = verbosity_objects(&value);
    assert_eq!(js_ts["unanalyzed_lines"], 0, "no count is known: {js_ts}");
    assert_eq!(js_ts["complete"], false, "{js_ts}");
    assert_eq!(java["complete"], true, "{java}");
}

#[test]
fn test_terminal_summary_carries_the_scores() {
    // The terminal summary (main.rs prints `report::terminal_summary`) had
    // zero test coverage: gutting it to an empty string kept the suite
    // green. Spawn the built binary, capture stdout, and assert it carries
    // the same numbers as that same run's own `report.json` — derived from
    // the JSON, not a pasted literal, so the two cannot silently diverge.
    let output_dir = tempfile::tempdir().expect("tempdir");
    let command_output = std::process::Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["scan", fixture_root().to_str().unwrap(), "--output"])
        .arg(output_dir.path())
        .output()
        .expect("spawn nsd");
    assert!(command_output.status.success());
    let stdout = String::from_utf8(command_output.stdout).expect("stdout is valid UTF-8");

    let json_text =
        fs::read_to_string(output_dir.path().join("report.json")).expect("report.json exists");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");

    let overall_erosion = value["scores"]["overall"]["erosion"].as_f64().unwrap();
    let overall_ratio = value["scores"]["overall"]["verbosity"]["ratio"]
        .as_f64()
        .unwrap();
    let java_erosion = value["scores"]["java"]["erosion"].as_f64().unwrap();
    let java_ratio = value["scores"]["java"]["verbosity"]["ratio"]
        .as_f64()
        .unwrap();
    let js_ts_erosion = value["scores"]["js_ts"]["erosion"].as_f64().unwrap();
    let js_ts_ratio = value["scores"]["js_ts"]["verbosity"]["ratio"]
        .as_f64()
        .unwrap();

    // Asserting the whole labelled line (not just a bare number that could
    // come from any family) pins each family to its own row: deleting a
    // family's writeln! block, or swapping which family's numbers get
    // printed on which line, both survived the bare-number checks this
    // replaces.
    assert!(
        stdout.contains(&format!(
            "  overall:  {overall_erosion:.4} / {overall_ratio:.4}\n"
        )),
        "stdout should carry the overall scores line: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  java:     {java_erosion:.4} / {java_ratio:.4}\n"
        )),
        "stdout should carry the java scores line: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  js_ts:    {js_ts_erosion:.4} / {js_ts_ratio:.4}\n"
        )),
        "stdout should carry the js_ts scores line: {stdout}"
    );
    assert!(
        stdout.contains("Incomplete (a file failed to parse): true"),
        "stdout should carry the incomplete marker: {stdout}"
    );
}

#[test]
fn test_terminal_summary_revision_line_has_no_dirty_suffix_for_local_targets() {
    // A local git work tree, clean or edited, prints `  revision: <sha>`
    // with no "(dirty: ...)" suffix; `dirty` is null in report.json.
    let repo_dir = tempfile::tempdir().expect("tempdir");
    init_git_worktree(repo_dir.path());
    fs::write(repo_dir.path().join("A.java"), "public class A {}\n").expect("write A.java");
    git_commit_all(repo_dir.path(), "initial commit");

    for edited in [false, true] {
        if edited {
            fs::write(
                repo_dir.path().join("A.java"),
                "public class A { void x() {} }\n",
            )
            .expect("edit A.java (uncommitted)");
        }
        let output_dir = tempfile::tempdir().expect("tempdir");
        let command_output = std::process::Command::new(env!("CARGO_BIN_EXE_nsd"))
            .args(["scan", repo_dir.path().to_str().unwrap(), "--output"])
            .arg(output_dir.path())
            .output()
            .expect("spawn nsd");
        assert!(command_output.status.success());
        let stdout = String::from_utf8(command_output.stdout).expect("stdout is valid UTF-8");

        let json_text =
            fs::read_to_string(output_dir.path().join("report.json")).expect("report.json exists");
        let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");
        let sha = value["scan"]["revision"]["sha"]
            .as_str()
            .expect("a git worktree scan publishes a sha")
            .to_string();
        assert!(
            value["scan"]["revision"]["dirty"].is_null(),
            "dirty is null (edited: {edited}): {value}"
        );
        assert!(
            stdout.contains(&format!("  revision: {sha}\n")),
            "stdout carries the bare sha (edited: {edited}): {stdout}"
        );
        let html =
            fs::read_to_string(output_dir.path().join("report.html")).expect("report.html exists");
        assert!(
            html.contains(&format!("<li>revision: <code>{sha}</code></li>")),
            "report.html carries the bare sha (edited: {edited})"
        );
        assert!(
            !html.contains("(dirty:"),
            "a local git target must not render a dirty suffix in report.html (edited: {edited})"
        );
        assert!(
            !stdout.contains("(dirty:"),
            "a local git target must not print a dirty suffix (edited: {edited}): {stdout}"
        );
    }

    // Non-git target: the revision line must render exactly as before --
    // no "(dirty: ...)" suffix at all, since `dirty` is `None` there.
    let non_git_dir = tempfile::tempdir().expect("tempdir");
    fs::write(non_git_dir.path().join("A.java"), "public class A {}\n")
        .expect("write A.java in a non-git directory");

    let non_git_output_dir = tempfile::tempdir().expect("tempdir");
    let non_git_command_output = std::process::Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["scan", non_git_dir.path().to_str().unwrap(), "--output"])
        .arg(non_git_output_dir.path())
        .output()
        .expect("spawn nsd");
    assert!(non_git_command_output.status.success());
    let non_git_stdout =
        String::from_utf8(non_git_command_output.stdout).expect("stdout is valid UTF-8");

    assert!(
        non_git_stdout.contains("  revision: not_a_git_repository\n"),
        "a non-git target's revision line must render unchanged: {non_git_stdout}"
    );
    assert!(
        !non_git_stdout.contains("(dirty:"),
        "a non-git target must not print a dirty suffix: {non_git_stdout}"
    );
}

#[test]
fn test_a_parse_failure_is_not_fatal() {
    // Exit code 0 despite the incomplete marker (D18): a parse failure
    // degrades the scan, it does not fail it.
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["scan", fixture_root().to_str().unwrap(), "--output"])
        .arg(tempfile::tempdir().expect("tempdir").path())
        .status()
        .expect("spawn nsd");
    assert!(status.success(), "a parse failure alone must not be fatal");
}

#[cfg(unix)]
#[test]
fn test_unwritable_output_directory_is_fatal() {
    // D18's third reserved fatal condition: an unwritable `--output`
    // directory. main.rs's `create_dir_all` is a no-op on an already-
    // existing directory, so the actual failure surfaces when `report::run`
    // tries to write `report.json` into it.
    use std::os::unix::fs::PermissionsExt;

    let output_dir = tempfile::tempdir().expect("tempdir");
    fs::set_permissions(output_dir.path(), fs::Permissions::from_mode(0o555))
        .expect("set read-only permissions");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["scan", fixture_root().to_str().unwrap(), "--output"])
        .arg(output_dir.path())
        .status()
        .expect("spawn nsd");

    // Restore the mode before the tempdir drops, so it can be cleaned up.
    fs::set_permissions(output_dir.path(), fs::Permissions::from_mode(0o755))
        .expect("restore permissions");

    assert!(
        !status.success(),
        "an unwritable --output directory must be fatal"
    );
}

#[test]
fn test_source_spans_match_the_inspected_code() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let report = &output.report;

    // Findings and duplicate locations carry a real, possibly multi-line
    // span (D22/D14): each one must bracket exactly the lines the rule (or
    // clone detector) actually flagged, read back off disk.
    for finding in &report.findings {
        assert_bracket_exact(&fixture_root(), &finding.location);
    }
    for group in &report.duplicates {
        for location in &group.locations {
            assert_bracket_exact(&fixture_root(), location);
        }
    }
}

/// M0c-13: Top-25 rows (D8's `Callable`) now publish the callable's own
/// declaration end line too (`IrCallable::span.end_line`, not just its
/// start line), so the excerpt brackets the whole declaration, not just its
/// first line (see docs/report-format.md's "Top-25 span" section, which
/// used to disclose the single-line compromise this closes).
#[test]
fn test_top25_span_covers_the_whole_callable() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let report = &output.report;

    assert!(!report.top25.is_empty());
    let mut any_multi_line_span = false;
    for callable in &report.top25 {
        assert!(
            callable.location.end_line >= callable.location.start_line,
            "a top-25 row's end_line must not precede its start_line: {callable:?}"
        );
        if callable.location.end_line > callable.location.start_line {
            any_multi_line_span = true;
        }
        assert_bracket_exact(&fixture_root(), &callable.location);
    }
    assert!(
        any_multi_line_span,
        "at least one top-25 row's fixture callable must span more than its \
         declaration line, or this test cannot discriminate end_line from \
         start_line: {:?}",
        report.top25
    );

    // The `>=` loop above only proves end_line never precedes start_line, so
    // it survives a mutant that adds a constant offset to end_line (e.g.
    // `start_line + 1`) instead of using the real declaration span. These
    // three exact pins, read by hand off the fixture files, catch that
    // mutant directly.
    let expected_spans = [
        ("classify", "src/Sample.java", 2usize, 8usize),
        ("orderSummary", "src/sample.js", 1usize, 12usize),
        (
            "unreachableDemo",
            "src/\"><img onerror=1>.js",
            1usize,
            5usize,
        ),
    ];
    for (name, relative_path, expected_start, expected_end) in expected_spans {
        let callable = report
            .top25
            .iter()
            .find(|c| c.name == name && c.location.relative_path == Path::new(relative_path))
            .unwrap_or_else(|| {
                panic!(
                    "{name} in {relative_path} should be in top25: {:?}",
                    report.top25
                )
            });
        assert_eq!(
            (callable.location.start_line, callable.location.end_line),
            (expected_start, expected_end),
            "{name} in {relative_path}: {callable:?}"
        );
    }
}

fn assert_bracket_exact(root: &Path, location: &report::SourceLocation) {
    let text = fs::read_to_string(root.join(&location.relative_path))
        .expect("fixture file backing a location should be readable");
    let lines: Vec<&str> = text.lines().collect();
    let expected = lines[location.start_line - 1..location.end_line].join("\n");
    assert_eq!(
        location.excerpt, expected,
        "{:?} does not bracket exactly the inspected code",
        location
    );
}

#[test]
fn test_remote_target_links_point_at_the_scanned_revision() {
    // No network access: `aggregate` is exercised directly with a
    // hand-crafted `Target::Remote` and a known sha, so the D6 GitHub
    // blob-URL construction is asserted deterministically.
    let target = Target::Remote(RemoteTarget {
        url: "https://github.com/an-owner/a-repo".to_string(),
    });
    let revision = Revision {
        sha: Some("deadbeefcafe".to_string()),
        dirty: Some(false),
        unavailable_reason: None,
    };
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let input = ReportInput {
        target: &target,
        target_input: "https://github.com/an-owner/a-repo",
        root: &fixture_root(),
        revision: &revision,
        settings: &output.settings,
        discover: &output.discover,
        parse_failures: &output.parse_failures,
        metrics: &output.metrics,
        clones: &output.clones,
        rules: &output.rules,
    };
    let report = report::aggregate(&input);
    let finding = report
        .findings
        .first()
        .expect("fixture should have at least one finding");
    assert!(finding.location.is_remote_link);
    // Pinned so the encoded literal below is checked against a known path:
    // `"` (0x22) sorts before `S`, so the XSS-payload-named fixture file is
    // deterministically `findings[0]`.
    assert_eq!(
        finding.location.relative_path,
        Path::new("src/\"><img onerror=1>.js")
    );
    // D6's blob URL percent-encodes each path segment (space, `"`, `<`, `>`
    // are not valid in a URL path): the raw filename must not appear.
    let expected_prefix =
        "https://github.com/an-owner/a-repo/blob/deadbeefcafe/src/%22%3E%3Cimg%20onerror%3D1%3E.js";
    assert!(
        finding.location.link.starts_with(expected_prefix),
        "{} should start with {expected_prefix}",
        finding.location.link
    );
    assert!(finding.location.link.ends_with(&format!(
        "#L{}-L{}",
        finding.location.start_line, finding.location.end_line
    )));
    let published: serde_json::Value =
        serde_json::from_str(&report::render_json(&report).expect("render")).expect("JSON");
    assert_eq!(
        published["scan"]["target"],
        "https://github.com/an-owner/a-repo"
    );
}

/// The keys of every object in `text`, in the order written, one list per
/// object (`serde_json::Value` would re-sort them, hiding the order).
fn key_orders(text: &str) -> Vec<Vec<String>> {
    let mut finished = Vec::new();
    let mut open: Vec<Vec<String>> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '{' => open.push(Vec::new()),
            '}' => finished.extend(open.pop()),
            '"' => {
                let mut literal = String::new();
                while let Some(next) = chars.next() {
                    match next {
                        '\\' => {
                            literal.push(next);
                            literal.extend(chars.next());
                        }
                        '"' => break,
                        other => literal.push(other),
                    }
                }
                if chars.peek() == Some(&':') {
                    open.last_mut()
                        .expect("a key sits in an object")
                        .push(literal);
                }
            }
            _ => {}
        }
    }
    finished
}

fn report_text(output: &PipelineOutput) -> String {
    fs::read_to_string(output.settings.output.join("report.json")).expect("report.json exists")
}

/// M6-1: the scan document is canonical: versioned, scoped, key-sorted.
#[test]
fn test_scan_report_is_canonical_with_scope_scan() {
    let (_dir, output) = run_scan(&fixture_root(), |_| {});
    let text = report_text(&output);
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    assert_eq!(value["schema_version"], 1, "{text}");
    assert_eq!(value["result_scope"], "scan", "{text}");
    let orders = key_orders(&text);
    assert!(orders.len() > 20, "the report nests many objects: {text}");
    for keys in &orders {
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, &sorted, "keys must be written sorted: {keys:?}");
    }
    assert!(
        text.contains("\"overall\": {"),
        "the pretty layout perf_scan.sh anchors on stays"
    );
}

/// M6-2: no excerpt key anywhere (findings and duplicates included), no
/// checkout path, no fixture source line.
#[test]
fn test_scan_report_holds_no_excerpt_or_absolute_path() {
    let root = fixture_root();
    let (_dir, output) = run_scan(&root, |_| {});
    let text = report_text(&output);

    assert!(
        !output.report.findings.is_empty() && !output.report.duplicates.is_empty(),
        "the fixture must exercise every location-bearing section"
    );
    assert!(!text.contains("\"excerpt\""), "{text}");
    for form in [
        root.to_str().expect("UTF-8 path").to_string(),
        root.canonicalize()
            .expect("canonicalize")
            .to_str()
            .expect("UTF-8 path")
            .to_string(),
    ] {
        assert!(!text.contains(&form), "the report names {form}");
    }
    let mut checked = 0;
    for file in ["Sample.java", "sample.js", "sample2.js", "Anonymous.js"] {
        let source = fs::read_to_string(root.join("src").join(file)).expect("read the fixture");
        for line in source.lines().map(str::trim).filter(|l| l.len() >= 12) {
            assert!(!text.contains(line), "{file} line {line:?} leaked");
            checked += 1;
        }
    }
    assert!(checked > 5, "the fixtures must supply source lines");
}

fn scan_value(root: &Path, configure: impl FnOnce(&mut ScanSettings)) -> serde_json::Value {
    let (_dir, output) = run_scan(root, configure);
    serde_json::from_str(&report_text(&output)).expect("valid JSON")
}

fn count_of(skipped_files: &[serde_json::Value], reason: &str) -> u64 {
    skipped_files
        .iter()
        .filter(|row| row["reason"] == reason)
        .count() as u64
}

/// One change to the scan settings.
type Mutation = dyn Fn(&mut ScanSettings);

/// Every `skipped_files` reason a scan can name: the discovery labels, then
/// the parse failures prefixed `parse_`.
const SCAN_SKIP_KEYS: [&str; 10] = [
    "dependency_or_build_output",
    "generated_code",
    "gitignore",
    "parse_grammar_setup",
    "parse_syntax_error",
    "parse_unreadable",
    "parse_unsupported_extension",
    "test",
    "unreadable",
    "user_exclude",
];

/// M6-1/M6-2: fingerprints, snapshot and per-reason skip counts.
#[test]
fn test_scan_report_carries_fingerprints_snapshot_and_skip_counts() {
    // Skips: a user exclude and a salvaged parse failure.
    let corpus = tempfile::tempdir().expect("tempdir");
    fs::write(corpus.path().join("A.java"), "class A { void a() {} }\n").expect("write");
    fs::write(
        corpus.path().join("Bad.java"),
        "public class Bad {\n    public void broken( {\n        return\n    }\n}\n",
    )
    .expect("write");
    fs::create_dir(corpus.path().join("custom")).expect("mkdir");
    fs::write(corpus.path().join("custom/V.java"), "class V {}\n").expect("write");
    let custom_excluded = |settings: &mut ScanSettings| {
        settings.exclude = vec!["**/custom/**".to_string()];
    };
    let value = scan_value(corpus.path(), custom_excluded);

    let skipped_files = value["skipped_files"].as_array().expect("skipped_files");
    let counts = value["skipped"].as_object().expect("skipped is an object");
    let keys: Vec<&str> = counts.keys().map(String::as_str).collect();
    assert_eq!(keys, SCAN_SKIP_KEYS, "every reason is present, zeros too");
    for key in SCAN_SKIP_KEYS {
        assert_eq!(
            counts[key].as_u64(),
            Some(count_of(skipped_files, key)),
            "{key}: {value}"
        );
    }
    assert_eq!(counts["parse_syntax_error"], 1, "{value}");
    assert_eq!(counts["user_exclude"], 1, "{value}");

    // Fingerprints: configuration moves with every scan setting.
    let configuration = |mutate: &Mutation| {
        let value = scan_value(corpus.path(), |settings| {
            custom_excluded(settings);
            mutate(settings);
        });
        value["fingerprints"]["configuration"].clone()
    };
    let base = configuration(&|_| {});
    let text = base.as_str().expect("configuration fingerprint");
    assert!(
        text.len() == "blake3:".len() + 32 && text.starts_with("blake3:"),
        "{text}"
    );
    assert_eq!(configuration(&|_| {}), base, "same settings, same value");
    let changes: [(&str, &Mutation); 3] = [
        ("include_tests", &|s| s.include_tests = !s.include_tests),
        ("exclude", &|s| s.exclude.push("**/x/**".to_string())),
        ("min_clone_lines", &|s| s.min_clone_lines = 77),
    ];
    for (name, mutate) in changes {
        assert_ne!(configuration(mutate), base, "{name} must move it");
    }

    // Snapshots: a git root has an ID, a subdirectory and a plain directory
    // have none and say why.
    assert_eq!(value["snapshots"]["scan"], serde_json::Value::Null);
    assert_eq!(
        value["snapshots"]["unavailable_reason"], "not_a_git_repository",
        "{value}"
    );
    let repo_dir = tempfile::tempdir().expect("tempdir");
    init_git_worktree(repo_dir.path());
    fs::write(repo_dir.path().join("A.java"), "public class A {}\n").expect("write A.java");
    fs::create_dir(repo_dir.path().join("sub")).expect("mkdir");
    fs::write(repo_dir.path().join("sub/B.java"), "public class B {}\n").expect("write B.java");
    git_commit_all(repo_dir.path(), "initial commit");

    let at_root = scan_value(repo_dir.path(), |_| {});
    let repo = Repository::open(repo_dir.path()).expect("open the repository");
    let expected = SnapshotId::of_worktree(&repo, &WorktreeSnapshot::open(&repo).expect("open"))
        .expect("worktree id")
        .to_string();
    assert_eq!(at_root["snapshots"]["scan"], expected, "{at_root}");
    assert_eq!(
        at_root["snapshots"]["unavailable_reason"],
        serde_json::Value::Null
    );
    let at_subdirectory = scan_value(&repo_dir.path().join("sub"), |_| {});
    assert_eq!(
        at_subdirectory["snapshots"]["scan"],
        serde_json::Value::Null
    );
    assert_eq!(
        at_subdirectory["snapshots"]["unavailable_reason"], "target_not_git_root",
        "{at_subdirectory}"
    );
}

/// D7: the measurement fingerprint is the run's `--min-clone-lines`, never
/// the default profile's.
#[test]
fn test_scan_measurement_fingerprint_uses_the_run_clone_threshold() {
    let measured = |min_clone_lines: u32| {
        let value = scan_value(&fixture_root(), |settings| {
            settings.min_clone_lines = min_clone_lines;
        });
        value["fingerprints"]["measurement"]
            .as_str()
            .expect("measurement fingerprint")
            .to_string()
    };
    let expected = |min_clone_lines: u32| {
        fingerprint(&MeasurementProfileInputs {
            min_clone_lines,
            ..MeasurementProfileInputs::current()
        })
    };

    let (ten, twenty) = (measured(10), measured(20));
    assert_eq!(ten, expected(10));
    assert_eq!(twenty, expected(20));
    assert_ne!(ten, twenty);
}

/// M6-2: the complete callable entity set is uncapped, sorted by path, start
/// line, end line, then name, and carries no excerpt.
#[test]
fn test_scan_report_lists_every_measured_callable() {
    let corpus = tempfile::tempdir().expect("tempdir");
    let method = |index: usize| format!("    int m{index}(int x) {{ return x + {index}; }}\n");
    let mut first = String::from("class Z {\n");
    first.extend((0..14).map(method));
    first.push_str("    void b() {} void a() {}\n}\n");
    let mut second = String::from("class Y {\n");
    second.extend((14..28).map(method));
    second.push_str("}\n");
    // `Z.java` sorts after `Y.java` though it is written first.
    fs::write(corpus.path().join("Z.java"), first).expect("write");
    fs::write(corpus.path().join("Y.java"), second).expect("write");

    let value = scan_value(corpus.path(), |_| {});

    let callables = value["callables"].as_array().expect("callables");
    assert_eq!(callables.len(), 30, "{value}");
    assert_eq!(value["top25"].as_array().expect("top25").len(), 25);
    let order: Vec<(String, u64, u64, String)> = callables
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().expect("path").to_string(),
                entry["start_line"].as_u64().expect("start"),
                entry["end_line"].as_u64().expect("end"),
                entry["name"].as_str().expect("name").to_string(),
            )
        })
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "path, start line, end line, then name");
    let tail: Vec<&str> = order[order.len() - 2..]
        .iter()
        .map(|entry| entry.3.as_str())
        .collect();
    assert_eq!(tail, ["a", "b"], "one start line is ordered by name");
    for entry in callables {
        let mut keys: Vec<&str> = entry
            .as_object()
            .expect("entry")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "cc",
                "end_line",
                "mass",
                "name",
                "path",
                "sloc",
                "start_line"
            ]
        );
    }
    for row in value["top25"].as_array().expect("top25") {
        let location = &row["location"];
        assert!(
            callables
                .iter()
                .any(|entry| entry["path"] == location["relative_path"]
                    && entry["start_line"] == location["start_line"]
                    && entry["name"] == row["name"]
                    && entry["cc"] == row["cc"]),
            "top25 row {row} is missing from callables"
        );
    }
}

/// A JS callable assigned to a computed member whose key expression holds a
/// secret; the assigned function starts on line 5.
const COMPUTED_MEMBER_PROBE: &str = "registry[(function () {\n  const apiKey = \"SECRET-TOKEN-1234\";\n  return apiKey;\n})()] =\nfunction (x) {\n  if (x) { return 1; }\n  return 2;\n};\n";

#[test]
fn test_scan_report_publishes_no_source_text_in_callable_names() {
    let corpus = tempfile::tempdir().expect("tempdir");
    fs::write(corpus.path().join("reg.js"), COMPUTED_MEMBER_PROBE).expect("write");

    let (_dir, output) = run_scan(corpus.path(), |_| {});
    let text = report_text(&output);
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    assert!(!text.contains("SECRET-TOKEN-1234"), "{text}");
    for section in ["callables", "top25"] {
        for row in value[section].as_array().expect("array") {
            let name = row["name"].as_str().expect("name");
            assert!(!name.contains(['\n', '\r']), "{section}: {name:?}");
        }
    }
    let names: Vec<&str> = value["callables"]
        .as_array()
        .expect("callables")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"<computed>@5"), "{names:?}");
}

fn callable_names(value: &serde_json::Value) -> Vec<String> {
    value["callables"]
        .as_array()
        .expect("callables")
        .iter()
        .filter_map(|row| row["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn test_scan_report_publishes_no_comment_or_pattern_text_in_callable_names() {
    let corpus = tempfile::tempdir().expect("tempdir");
    fs::write(
        corpus.path().join("names.js"),
        "obj /* SECRET-COMMENT-5678 */ .handler = function (x) {\n  if (x) { return 1; }\n  return 2;\n};\nconst { k = \"SECRET-DEFAULT-9999\" } = function (x) {\n  if (x) { return 1; }\n  return 2;\n};\n",
    )
    .expect("write");

    let (_dir, output) = run_scan(corpus.path(), |_| {});
    let text = report_text(&output);
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    assert!(!text.contains("SECRET-COMMENT-5678"), "{text}");
    assert!(!text.contains("SECRET-DEFAULT-9999"), "{text}");
    let names = callable_names(&value);
    assert!(names.contains(&"<computed>@1".to_string()), "{names:?}");
    assert!(names.contains(&"<computed>@5".to_string()), "{names:?}");
}

#[test]
fn test_scan_report_replaces_bracket_only_and_line_break_only_names() {
    let corpus = tempfile::tempdir().expect("tempdir");
    fs::write(
        corpus.path().join("one.js"),
        "registry[\"SECRET-TOKEN-5678\"] = function (x) {\n  if (x) { return 1; }\n  return 2;\n};\n",
    )
    .expect("write");
    fs::write(
        corpus.path().join("two.js"),
        "registry\n  .handler = function (x) {\n  if (x) { return 1; }\n  return 2;\n};\n",
    )
    .expect("write");

    let (_dir, output) = run_scan(corpus.path(), |_| {});
    let text = report_text(&output);
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    assert!(!text.contains("SECRET-TOKEN-5678"), "{text}");
    let names = callable_names(&value);
    assert!(names.contains(&"<computed>@1".to_string()), "{names:?}");
    assert!(names.contains(&"<computed>@2".to_string()), "{names:?}");
}

#[test]
fn test_scan_report_orders_callables_with_one_start_line_by_end_line() {
    let corpus = tempfile::tempdir().expect("tempdir");
    fs::write(
        corpus.path().join("T.java"),
        "class T {\nvoid d() {} void c() {\nint x = 1;\n}\n}\n",
    )
    .expect("write");

    let value = scan_value(corpus.path(), |_| {});

    let names: Vec<&str> = value["callables"]
        .as_array()
        .expect("callables")
        .iter()
        .map(|row| row["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["d", "c"], "{value}");
}

fn scan_id(root: &Path) -> String {
    scan_value(root, |_| {})["snapshots"]["scan"]
        .as_str()
        .expect("a string snapshot ID")
        .to_string()
}

/// D22: the scan ID covers tracked entries plus untracked non-ignored ones.
/// Rejects an ID that hashes an ignored untracked tree (the walk that ignores
/// `.gitignore`), one that drops a tracked file matching an ignore pattern,
/// and one that drops untracked non-ignored files.
#[test]
fn test_scan_snapshot_id_skips_ignored_untracked_entries() {
    let repo_dir = tempfile::tempdir().expect("tempdir");
    let root = repo_dir.path();
    init_git_worktree(root);
    fs::write(root.join(".gitignore"), "build/\n*.log\n").expect("write .gitignore");
    fs::write(root.join("A.java"), "public class A {}\n").expect("write A.java");
    fs::write(root.join("kept.log"), "tracked despite the pattern\n").expect("write kept.log");
    run_git(root, &["add", "-A"]);
    run_git(root, &["add", "-f", "kept.log"]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::create_dir(root.join("build")).expect("mkdir build");
    fs::write(root.join("build/out.bin"), "one").expect("write out.bin");
    fs::write(root.join("stray.log"), "ignored untracked file").expect("write stray.log");

    let base = scan_id(root);

    // (a) An ignored untracked file or directory entry never moves the ID.
    fs::write(root.join("build/out.bin"), "two").expect("rewrite out.bin");
    fs::write(root.join("build/more.bin"), "new").expect("write more.bin");
    fs::write(root.join("stray.log"), "changed").expect("rewrite stray.log");
    assert_eq!(scan_id(root), base, "ignored entries must not move the ID");

    // (f) Without ignored entries the ID is check's worktree ID.
    fs::remove_dir_all(root.join("build")).expect("remove build");
    fs::remove_file(root.join("stray.log")).expect("remove stray.log");
    let repo = Repository::open(root).expect("open the repository");
    let worktree_id = SnapshotId::of_worktree(&repo, &WorktreeSnapshot::open(&repo).expect("open"))
        .expect("worktree id")
        .to_string();
    assert_eq!(scan_id(root), worktree_id);
    assert_eq!(scan_id(root), base, "the ignored entries never counted");

    // (d) A tracked file matching an ignore pattern still counts.
    fs::write(root.join("kept.log"), "tracked, edited").expect("edit kept.log");
    let after_kept = scan_id(root);
    assert_ne!(after_kept, base, "a tracked ignored-pattern file counts");

    // (b) A tracked source change moves the ID.
    fs::write(root.join("A.java"), "public class A { int x; }\n").expect("edit A.java");
    let after_tracked = scan_id(root);
    assert_ne!(after_tracked, after_kept, "a tracked edit moves the ID");

    // (c) An untracked, non-ignored file moves the ID.
    fs::write(root.join("new.java"), "public class N {}\n").expect("write new.java");
    assert_ne!(scan_id(root), after_tracked, "an untracked file counts");
}

/// D22: an ignored untracked directory is pruned, never opened.
/// Rejects a walk that stats or hashes files inside an ignored tree.
#[cfg(unix)]
#[test]
fn test_scan_snapshot_id_never_opens_an_ignored_directory() {
    use std::os::unix::fs::PermissionsExt;

    let repo_dir = tempfile::tempdir().expect("tempdir");
    let root = repo_dir.path();
    init_git_worktree(root);
    fs::write(root.join(".gitignore"), "build/\n").expect("write .gitignore");
    fs::write(root.join("A.java"), "public class A {}\n").expect("write A.java");
    git_commit_all(root, "initial commit");
    fs::create_dir_all(root.join("build/locked")).expect("mkdir");
    let unreadable = root.join("build/out.bin");
    fs::write(&unreadable, "x").expect("write out.bin");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmod file");
    let locked = root.join("build/locked");
    fs::write(locked.join("inner.bin"), "y").expect("write inner.bin");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod dir");

    let value = scan_value(root, |_| {});
    // Restore access so the temp dir can be removed.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore dir");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("restore file");
    assert!(value["snapshots"]["scan"].is_string(), "{value}");
    assert_eq!(
        value["snapshots"]["unavailable_reason"],
        serde_json::Value::Null
    );
}

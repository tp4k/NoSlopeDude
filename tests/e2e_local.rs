//! WS-6 local end-to-end test: `pipeline::run` (the exact function
//! `main.rs` calls for the CLI's `scan` subcommand) scans a small,
//! hand-authored multi-language repository, and the test reads
//! `report.json` back and cross-checks `PipelineOutput`'s own stage
//! results against independently, by-hand counted fixture facts: 3
//! discovered files, 3 callables (one per file, CC/SLOC hand-computed per
//! `docs/cc-rules.md`), and one clone group of exactly 10 redundant lines
//! between the two JS files (per `docs/clone-detection.md`'s D11 rule).

use std::fs;
use std::path::{Path, PathBuf};

use nsd::model::{ScanSettings, DEFAULT_MIN_CLONE_LINES};
use nsd::pipeline::{self, PipelineOutput};
use nsd::report::SourceLocation;

/// One `if`, base CC 1 -> `cc == 2`. Three executable body lines (the `if`
/// header and the two `return`s; both `}` lines are anonymous-only and do
/// not count) -> `sloc == 3`.
const GREETER_JAVA: &str = "public class Greeter {\n    public String greet(String name) {\n        if (name == null) {\n            return \"Hello, World!\";\n        }\n        return \"Hello, \" + name + \"!\";\n    }\n}\n";

/// One `for`, base CC 1 -> `cc == 2`. Ten executable body lines (every
/// statement line plus the `for` header; the loop's closing `}` and the
/// function's own braces are anonymous-only and do not count) -> `sloc ==
/// 10`, which is also this file's clone-candidate D11 size (the whole
/// function body is one candidate, the maximal one), exactly at
/// `DEFAULT_MIN_CLONE_LINES`.
const DUPLICATE_JS: &str = "function process(items) {\n  let total = 0;\n  let count = 0;\n  for (let i = 0; i < items.length; i++) {\n    total = total + items[i];\n    count = count + 1;\n  }\n  let average = total / count;\n  let rounded = Math.round(average * 100) / 100;\n  let label = \"avg: \" + rounded;\n  console.log(label);\n  return rounded;\n}\n";

/// Writes the fixture's three files under `root/src/`.
fn write_fixture(root: &Path) {
    let src = root.join("src");
    fs::create_dir_all(&src).expect("create src dir");
    fs::write(src.join("Greeter.java"), GREETER_JAVA).expect("write Greeter.java");
    fs::write(src.join("duplicate_a.js"), DUPLICATE_JS).expect("write duplicate_a.js");
    fs::write(src.join("duplicate_b.js"), DUPLICATE_JS).expect("write duplicate_b.js");
}

/// Generates the fixture into a fresh temp directory, scans it through the
/// same pipeline entry point `main.rs` uses, and keeps both temp
/// directories (fixture root and report output) alive for the caller.
fn run_scan() -> (tempfile::TempDir, tempfile::TempDir, PipelineOutput) {
    let fixture_dir = tempfile::tempdir().expect("fixture tempdir");
    write_fixture(fixture_dir.path());
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
    let output = pipeline::run(&target_input, settings).expect("pipeline run should succeed");
    (fixture_dir, output_dir, output)
}

#[test]
fn test_local_repo_scan_totals_agree_with_fixture_facts() {
    let (_fixture_dir, output_dir, output) = run_scan();

    // Independently counted fixture fact: exactly 3 files were authored,
    // none of them matches a default exclusion (D16).
    assert!(
        output.discover.skipped.is_empty(),
        "the fixture has no test/generated/excluded file: {:?}",
        output.discover.skipped
    );
    let mut discovered: Vec<PathBuf> = output
        .discover
        .discovered
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    discovered.sort();
    assert_eq!(
        discovered,
        vec![
            PathBuf::from("src/Greeter.java"),
            PathBuf::from("src/duplicate_a.js"),
            PathBuf::from("src/duplicate_b.js"),
        ],
        "expected exactly the 3 authored files to be discovered"
    );

    let json_text =
        fs::read_to_string(output_dir.path().join("report.json")).expect("report.json exists");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");

    // Independently counted fixture fact: 1 callable per file == 3.
    let top25 = value["top25"].as_array().expect("top25 array");
    assert_eq!(top25.len(), 3, "expected one callable per file: {top25:?}");

    let greet = top25
        .iter()
        .find(|callable| callable["name"] == "greet")
        .expect("greet callable should be reported");
    assert_eq!(
        greet["cc"].as_u64().unwrap(),
        2,
        "greet: 1 base + 1 if == 2"
    );
    assert_eq!(
        greet["sloc"].as_u64().unwrap(),
        3,
        "greet's 3 executable body lines"
    );

    let processes: Vec<&serde_json::Value> = top25
        .iter()
        .filter(|callable| callable["name"] == "process")
        .collect();
    assert_eq!(
        processes.len(),
        2,
        "both duplicate files declare a process callable"
    );
    for process in processes {
        assert_eq!(
            process["cc"].as_u64().unwrap(),
            2,
            "process: 1 base + 1 for == 2"
        );
        assert_eq!(
            process["sloc"].as_u64().unwrap(),
            10,
            "process's 10 executable body lines"
        );
    }

    // Independently counted fixture fact: the known clone group.
    let duplicates = value["duplicates"].as_array().expect("duplicates array");
    assert_eq!(
        duplicates.len(),
        1,
        "exactly one clone group between the two JS files: {duplicates:?}"
    );
    let group = &duplicates[0];
    assert_eq!(
        group["redundant_lines"].as_u64().unwrap(),
        10,
        "one redundant 10-line occurrence beyond the first"
    );
    let locations = group["locations"].as_array().expect("locations array");
    assert_eq!(locations.len(), 2, "the clone spans both duplicate files");
    let mut paths: Vec<&str> = locations
        .iter()
        .map(|location| location["relative_path"].as_str().unwrap())
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["src/duplicate_a.js", "src/duplicate_b.js"]);
}

#[test]
fn test_report_source_spans_resolve_on_disk() {
    let (fixture_dir, _output_dir, output) = run_scan();
    let report = &output.report;

    let mut checked = 0;
    for finding in &report.findings {
        assert_span_reads_back(fixture_dir.path(), &finding.location);
        checked += 1;
    }
    for group in &report.duplicates {
        for location in &group.locations {
            assert_span_reads_back(fixture_dir.path(), location);
            checked += 1;
        }
    }
    for callable in &report.top25 {
        assert_span_reads_back(fixture_dir.path(), &callable.location);
        checked += 1;
    }
    assert!(
        checked >= 5,
        "expected at least the 3 top-25 spans and the 2 clone-group \
         locations to be checked, got {checked}"
    );
}

/// Reads `location`'s span back off `root` and checks the excerpt the
/// report published brackets exactly the lines the scanner claims.
fn assert_span_reads_back(root: &Path, location: &SourceLocation) {
    let text = fs::read_to_string(root.join(&location.relative_path))
        .expect("fixture file backing a location should be readable");
    let lines: Vec<&str> = text.lines().collect();
    let expected = lines[location.start_line - 1..location.end_line].join("\n");
    assert_eq!(
        location.excerpt, expected,
        "{location:?} does not bracket exactly the fixture's own lines"
    );
}

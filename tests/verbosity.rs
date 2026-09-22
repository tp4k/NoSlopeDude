use std::path::{Path, PathBuf};

use nsd::model::{
    CloneGroup, CloneLocation, ClonesResult, DiscoveredFile, FileLanguageLines, LanguageFamily,
    RuleFinding,
};
use nsd::parse;
use nsd::{metrics, rules};

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules")
}

fn sample_path() -> PathBuf {
    PathBuf::from("Sample.java")
}

fn finding(
    rule_id: &'static str,
    start_line: usize,
    end_line: usize,
    flagged_lines: Vec<usize>,
) -> RuleFinding {
    RuleFinding {
        relative_path: sample_path(),
        language: JAVA,
        rule_id,
        start_line,
        end_line,
        flagged_lines,
    }
}

fn clone_location(
    path: &str,
    start_line: usize,
    end_line: usize,
    source_lines: usize,
) -> CloneLocation {
    CloneLocation {
        relative_path: PathBuf::from(path),
        start_line,
        end_line,
        source_lines,
    }
}

#[test]
fn test_verbosity_is_distinct_flagged_lines_over_scanned_lines() {
    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        executable_lines: (1..=100).collect(),
    }];
    let findings = vec![finding(
        rules::JAVA_EMPTY_CATCH,
        10,
        14,
        (10..=14).collect(),
    )];
    let group = CloneGroup {
        language: JAVA,
        locations: vec![
            clone_location("Sample.java", 12, 21, 10),
            clone_location("Sample.java", 30, 39, 10),
        ],
        redundant_lines: 10,
    };

    let verbosity = rules::compute_verbosity(&files, &findings, &[group]);

    assert_eq!(verbosity.overall.flagged_lines, 15, "{verbosity:?}");
    assert_eq!(verbosity.overall.scanned_lines, 100);
    assert_eq!(verbosity.overall.ratio, 0.15);
}

#[test]
fn test_overlapping_rule_and_clone_lines_counted_once() {
    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        executable_lines: (1..=100).collect(),
    }];
    let findings = vec![finding(
        rules::JAVA_EMPTY_CATCH,
        36,
        45,
        (36..=45).collect(),
    )];
    let group = CloneGroup {
        language: JAVA,
        locations: vec![
            clone_location("Sample.java", 1, 10, 10),
            clone_location("Sample.java", 30, 39, 10),
        ],
        redundant_lines: 10,
    };

    let verbosity = rules::compute_verbosity(&files, &findings, &[group]);

    assert_eq!(verbosity.overall.flagged_lines, 16, "{verbosity:?}");
    assert_eq!(verbosity.overall.ratio, 0.16);
}

#[test]
fn test_first_clone_occurrence_contributes_no_lines() {
    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        executable_lines: (1..=100).collect(),
    }];
    let group = CloneGroup {
        language: JAVA,
        locations: vec![
            clone_location("Sample.java", 1, 10, 10),
            clone_location("Sample.java", 50, 59, 10),
        ],
        redundant_lines: 10,
    };

    let verbosity = rules::compute_verbosity(&files, &[], &[group]);

    assert_eq!(verbosity.overall.flagged_lines, 10, "{verbosity:?}");
    assert_eq!(verbosity.overall.ratio, 0.10);
}

#[test]
fn test_verbosity_denominator_is_scanned_lines_not_callable_sloc() {
    let root = fixture_root();
    let discovered = vec![DiscoveredFile {
        relative_path: PathBuf::from("__tests__/ImportsAndFields.java"),
        language: JAVA,
    }];
    let (parsed, failures) = parse::parse_all(&root, &discovered);
    assert!(failures.is_empty(), "{failures:?}");

    let metrics_result = metrics::run(&parsed, false);
    let clones_result = ClonesResult { groups: Vec::new() };
    let rules_result = rules::run(&parsed, &metrics_result, &clones_result);

    let callable_sloc: usize = metrics_result.callables.iter().map(|c| c.sloc).sum();
    assert!(
        rules_result.verbosity.overall.scanned_lines > callable_sloc,
        "scanned={} sloc={} (imports/fields sit outside every callable, D12)",
        rules_result.verbosity.overall.scanned_lines,
        callable_sloc
    );
}

#[test]
fn test_unparsed_files_excluded_from_denominator_and_marked_incomplete() {
    let root = fixture_root();
    let discovered = vec![
        DiscoveredFile {
            relative_path: PathBuf::from("broken/Good.java"),
            language: JAVA,
        },
        DiscoveredFile {
            relative_path: PathBuf::from("broken/Broken.java"),
            language: JAVA,
        },
    ];
    let (parsed, failures) = parse::parse_all(&root, &discovered);
    assert_eq!(failures.len(), 1, "{failures:?}");

    let metrics_result = metrics::run(&parsed, !failures.is_empty());
    let clones_result = ClonesResult { groups: Vec::new() };
    let rules_result = rules::run(&parsed, &metrics_result, &clones_result);

    assert!(rules_result.incomplete);
    let good_summary = metrics_result
        .file_scan_summaries
        .iter()
        .find(|summary| summary.relative_path == Path::new("broken/Good.java"))
        .expect("Good.java should have parsed and been scanned");
    assert_eq!(
        rules_result.verbosity.overall.scanned_lines, good_summary.scanned_lines,
        "the broken file must not contribute to the denominator"
    );
}

#[test]
fn test_verbosity_zero_when_nothing_flagged() {
    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        executable_lines: (1..=100).collect(),
    }];
    let verbosity = rules::compute_verbosity(&files, &[], &[]);
    assert_eq!(verbosity.overall.ratio, 0.0);

    let empty_scan = rules::compute_verbosity(&[], &[], &[]);
    assert_eq!(empty_scan.overall.scanned_lines, 0);
    assert_eq!(
        empty_scan.overall.ratio, 0.0,
        "must not divide by zero on an empty scan"
    );
}

#[test]
fn test_per_family_and_overall_scores_are_computed_separately() {
    let files = vec![
        FileLanguageLines {
            relative_path: PathBuf::from("A.java"),
            language: JAVA,
            scanned_lines: 50,
            executable_lines: (1..=50).collect(),
        },
        FileLanguageLines {
            relative_path: PathBuf::from("B.js"),
            language: JS_TS,
            scanned_lines: 50,
            executable_lines: (1..=50).collect(),
        },
    ];
    let findings = vec![RuleFinding {
        relative_path: PathBuf::from("A.java"),
        language: JAVA,
        rule_id: rules::JAVA_EMPTY_CATCH,
        start_line: 1,
        end_line: 5,
        flagged_lines: (1..=5).collect(),
    }];

    let verbosity = rules::compute_verbosity(&files, &findings, &[]);

    assert_eq!(verbosity.java.flagged_lines, 5);
    assert_eq!(verbosity.java.ratio, 0.1);
    assert_eq!(verbosity.js_ts.flagged_lines, 0);
    assert_eq!(verbosity.js_ts.ratio, 0.0);
    assert_eq!(verbosity.overall.flagged_lines, 5);
    assert_eq!(verbosity.overall.ratio, 0.05);
}

#[test]
fn test_clone_occurrence_excludes_comment_and_blank_lines_from_numerator() {
    let root = fixture_root();
    let discovered = vec![DiscoveredFile {
        relative_path: PathBuf::from("__tests__/ClonedBlockWithGaps.java"),
        language: JAVA,
    }];
    let (parsed, failures) = parse::parse_all(&root, &discovered);
    assert!(failures.is_empty(), "{failures:?}");

    let metrics_result = metrics::run(&parsed, false);
    // The group's canonical first occurrence (`first()`'s body, lines 6-7)
    // contributes nothing (D23); the second occurrence (`second()`'s body,
    // lines 11-14) straddles a comment-only line (12) and a blank line
    // (13). D11 excludes both from the D12 line-count rule, so they must
    // not enter the verbosity numerator either, even though they sit
    // inside a counted occurrence's raw span.
    let group = CloneGroup {
        language: JAVA,
        locations: vec![
            clone_location("__tests__/ClonedBlockWithGaps.java", 6, 7, 2),
            clone_location("__tests__/ClonedBlockWithGaps.java", 11, 14, 2),
        ],
        redundant_lines: 2,
    };
    let clones_result = ClonesResult {
        groups: vec![group],
    };

    let rules_result = rules::run(&parsed, &metrics_result, &clones_result);

    assert!(
        rules_result.findings.is_empty(),
        "fixture must trigger no rule, isolating the clone-only numerator: {:?}",
        rules_result.findings
    );
    // Expected vs actual, the two edge cases this test pins: a naive
    // [start_line, end_line] span would count all 4 lines (11..=14) for
    // 4/8 = 0.5; the D11-correct answer counts only the two executable
    // lines (11 and 14) for 2/8 = 0.25.
    assert_eq!(
        rules_result.verbosity.overall.flagged_lines, 2,
        "{:?}",
        rules_result.verbosity
    );
    assert_eq!(rules_result.verbosity.overall.scanned_lines, 8);
    assert_eq!(rules_result.verbosity.overall.ratio, 0.25);
}

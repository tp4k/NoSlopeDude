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
        unanalyzed_lines: 0,
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
        unanalyzed_lines: 0,
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
        unanalyzed_lines: 0,
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

    // WS-6 declared delta: `Broken.java` used to be dropped wholesale (never
    // parsed, never lowered, contributing nothing anywhere), so it never
    // reached `file_scan_summaries` at all. Salvage means it is now lowered
    // like any other file -- its one callable (`method`) intersects the
    // damage and is pruned to an empty IR node (fail-closed, contributing
    // no executable lines of its own), but D12's per-file scanned-line count
    // is unconditioned on callable boundaries (`metrics::scan_file`'s own
    // doc comment) and still walks whatever survives pruning around it: the
    // class wrapper's own lines. `broken_summary.scanned_lines` documents
    // that contribution directly rather than pinning a bare "+2".
    let broken_summary = metrics_result
        .file_scan_summaries
        .iter()
        .find(|summary| summary.relative_path == Path::new("broken/Broken.java"))
        .expect("Broken.java should still reach the metrics stage under salvage");
    // Pinned to the fixture's actual, literal value -- not derived from the
    // same computation the sum-identity assertion below re-derives it from,
    // so that assertion cannot hold vacuously for a broken pruning that
    // still contributes *some* value here (e.g. counting the pruned
    // callable's own lines back in). `broken/Broken.java` (package + blank
    // + class decl + blank + damaged `method` + `}` x2) survives pruning
    // with exactly its package and class-declaration lines counted: 2.
    assert_eq!(
        broken_summary.scanned_lines, 2,
        "Broken.java's surviving (non-pruned) scanned-line count changed -- \
         update this pin only after confirming the new value is still exactly \
         the class wrapper's lines, not the damaged callable's own"
    );
    assert!(
        metrics_result
            .callables
            .iter()
            .all(|callable| callable.relative_path != Path::new("broken/Broken.java")),
        "Broken.java's one callable intersects the damage and must stay unmeasured: {:?}",
        metrics_result.callables
    );
    assert_eq!(
        rules_result.verbosity.overall.scanned_lines,
        good_summary.scanned_lines + broken_summary.scanned_lines,
        "the denominator is the sum of every salvage-parsed file's own scanned lines, \
         not just Good.java's -- Broken.java's damaged callable itself still \
         contributes none of its own (fail-closed), only its enclosing class wrapper's"
    );
}

#[test]
fn test_verbosity_zero_when_nothing_flagged() {
    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        unanalyzed_lines: 0,
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

/// M0c-13: `FileLanguageLines::executable_lines` moved from `BTreeSet<usize>`
/// to a sorted, deduplicated `Vec<usize>` (`compute_verbosity`'s own
/// membership check moved from `.contains()` to `.binary_search().is_ok()`
/// to match) — `binary_search` only gives a correct answer over an
/// already-sorted, duplicate-free slice. This test pins that
/// `binary_search` lookup against a genuinely sparse set with gaps, matching
/// what `executable_lines_from_ir` produces; the builder's own sorted/dedup
/// invariant (that it never hands `compute_verbosity` an unsorted or
/// duplicate-containing Vec in the first place) is pinned separately by
/// `rules::tests::test_file_language_lines_come_from_ir_spans`.
#[test]
fn test_executable_lines_are_sorted_and_distinct() {
    let executable_lines = vec![2usize, 5, 9, 40];

    let files = vec![FileLanguageLines {
        relative_path: sample_path(),
        language: JAVA,
        scanned_lines: 100,
        unanalyzed_lines: 0,
        executable_lines,
    }];
    let group = CloneGroup {
        language: JAVA,
        locations: vec![
            clone_location("Sample.java", 1, 10, 10),
            clone_location("Sample.java", 1, 45, 10),
        ],
        redundant_lines: 10,
    };

    let verbosity = rules::compute_verbosity(&files, &[], &[group]);

    // Expected vs actual, the two edge cases this test pins: a naive raw
    // [1, 45] span would count all 45 lines; the D11-correct answer counts
    // only the 4 sparse executable lines (2, 5, 9, 40) that fall inside it.
    assert_eq!(verbosity.overall.flagged_lines, 4, "{verbosity:?}");
}

#[test]
fn test_per_family_and_overall_scores_are_computed_separately() {
    let files = vec![
        FileLanguageLines {
            relative_path: PathBuf::from("A.java"),
            language: JAVA,
            scanned_lines: 50,
            unanalyzed_lines: 0,
            executable_lines: (1..=50).collect(),
        },
        FileLanguageLines {
            relative_path: PathBuf::from("B.js"),
            language: JS_TS,
            scanned_lines: 50,
            unanalyzed_lines: 0,
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

use std::path::PathBuf;

use nsd::clones::{self, redundant_occurrences};
use nsd::model::{DiscoveredFile, LanguageFamily, ScanSettings};
use nsd::parse::{self, ParsedFile};
use nsd::pipeline;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clones")
}

/// Parses every `(relative path, language)` pair under `tests/fixtures/clones/`.
fn parsed_files(paths: &[(&str, LanguageFamily)]) -> Vec<ParsedFile> {
    let root = fixture_root();
    let files: Vec<DiscoveredFile> = paths
        .iter()
        .map(|(path, language)| DiscoveredFile {
            relative_path: PathBuf::from(*path),
            language: *language,
        })
        .collect();
    let (parsed, failures) = parse::parse_all(&root, &files);
    assert!(
        failures.is_empty(),
        "unexpected parse failures: {failures:?}"
    );
    parsed
}

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

#[test]
fn test_identical_blocks_group_across_files() {
    let files = parsed_files(&[
        ("__tests__/DupIdenticalA.java", JAVA),
        ("__tests__/DupIdenticalB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 1, "{:?}", result.groups);
    let group = &result.groups[0];
    assert_eq!(group.locations.len(), 2, "{:?}", group.locations);
    for location in &group.locations {
        assert!(location.start_line > 0);
        assert!(location.end_line >= location.start_line);
    }
    let mut paths: Vec<&PathBuf> = group.locations.iter().map(|l| &l.relative_path).collect();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            &PathBuf::from("__tests__/DupIdenticalA.java"),
            &PathBuf::from("__tests__/DupIdenticalB.java"),
        ]
    );
    // Pin the 1-based line conversion (review-ws3-r1-code.md row 2): hand
    // counted off the fixtures, `locations` is already in canonical
    // (path, start line) order so A (alphabetically first) is index 0.
    let a = &group.locations[0];
    assert_eq!(
        a.relative_path,
        PathBuf::from("__tests__/DupIdenticalA.java")
    );
    assert_eq!(a.start_line, 3);
    assert_eq!(a.end_line, 14);
    assert_eq!(a.source_lines, 12);
    let b = &group.locations[1];
    assert_eq!(
        b.relative_path,
        PathBuf::from("__tests__/DupIdenticalB.java")
    );
    assert_eq!(b.start_line, 4);
    assert_eq!(b.end_line, 18);
    assert_eq!(b.source_lines, 12);
}

#[test]
fn test_identifier_change_breaks_the_group() {
    let files = parsed_files(&[
        ("__tests__/IdentifierA.java", JAVA),
        ("__tests__/IdentifierB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 0, "{:?}", result.groups);
}

#[test]
fn test_literal_change_breaks_the_group() {
    let files = parsed_files(&[
        ("__tests__/LiteralA.java", JAVA),
        ("__tests__/LiteralB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 0, "{:?}", result.groups);
}

#[test]
fn test_min_two_statements_enforced() {
    let files = parsed_files(&[
        ("__tests__/SingleStatementDupA.java", JAVA),
        ("__tests__/SingleStatementDupB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(
        result.groups.len(),
        0,
        "a single repeated statement must never become a clone group: {:?}",
        result.groups
    );
}

#[test]
fn test_min_clone_lines_threshold_applies() {
    let files = parsed_files(&[
        ("__tests__/DupIdenticalA.java", JAVA),
        ("__tests__/DupIdenticalB.java", JAVA),
    ]);
    let at_ten = clones::run(&files, 10);
    assert_eq!(at_ten.groups.len(), 1, "{:?}", at_ten.groups);

    let at_twenty = clones::run(&files, 20);
    assert_eq!(
        at_twenty.groups.len(),
        0,
        "raising --min-clone-lines above the duplicate's size must drop it: {:?}",
        at_twenty.groups
    );
}

#[test]
fn test_duplicated_switch_arms_group() {
    let java_files = parsed_files(&[("__tests__/SwitchDupJava.java", JAVA)]);
    let java_result = clones::run(&java_files, 10);
    assert_eq!(java_result.groups.len(), 1, "{:?}", java_result.groups);
    assert_eq!(java_result.groups[0].locations.len(), 2);
    // D20: the group's published language family (review-ws3-r1-code.md row 3).
    assert_eq!(java_result.groups[0].language, LanguageFamily::Java);

    let js_files = parsed_files(&[("__tests__/SwitchDupJs.js", JS_TS)]);
    let js_result = clones::run(&js_files, 10);
    assert_eq!(js_result.groups.len(), 1, "{:?}", js_result.groups);
    assert_eq!(js_result.groups[0].language, LanguageFamily::JsTs);
    // review-ws3-r1-code.md row 4: the switch_default arm now carries the
    // same run as the two switch_case arms, so it is a third location.
    assert_eq!(js_result.groups[0].locations.len(), 3);
}

/// D15's container list also covers a Java `constructor_body` and a JS/TS
/// top-level `program` — two containers the round-1 enumeration missed
/// (review-ws3-r1-code.md row 1): a duplicated constructor body and a
/// duplicated module-level statement run must each be found exactly like
/// the identical block inside a method already is.
#[test]
fn test_constructor_body_and_module_level_runs_group() {
    let java_files = parsed_files(&[
        ("__tests__/ConstructorBodyA.java", JAVA),
        ("__tests__/ConstructorBodyB.java", JAVA),
    ]);
    let java_result = clones::run(&java_files, 10);
    assert_eq!(java_result.groups.len(), 1, "{:?}", java_result.groups);
    assert_eq!(java_result.groups[0].locations.len(), 2);

    let js_files = parsed_files(&[
        ("__tests__/ModuleLevelA.js", JS_TS),
        ("__tests__/ModuleLevelB.js", JS_TS),
    ]);
    let js_result = clones::run(&js_files, 10);
    assert_eq!(js_result.groups.len(), 1, "{:?}", js_result.groups);
    assert_eq!(js_result.groups[0].locations.len(), 2);
}

#[test]
fn test_groups_ranked_by_redundant_lines_beyond_first() {
    let files = parsed_files(&[
        ("__tests__/RankGX1.java", JAVA),
        ("__tests__/RankGX2.java", JAVA),
        ("__tests__/RankGX3.java", JAVA),
        ("__tests__/RankGY1.java", JAVA),
        ("__tests__/RankGY2.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 2, "{:?}", result.groups);

    let top = &result.groups[0];
    let second = &result.groups[1];
    assert_eq!(top.locations.len(), 3, "{:?}", top.locations);
    assert_eq!(second.locations.len(), 2, "{:?}", second.locations);
    // 3 occurrences x 10 lines: 2 occurrences beyond the first = 20.
    assert_eq!(top.redundant_lines, 20);
    // 2 occurrences x 15 lines: 1 occurrence beyond the first = 15.
    assert_eq!(second.redundant_lines, 15);
    assert!(
        top.redundant_lines > second.redundant_lines,
        "the 3x10 group must outrank the 2x15 group despite its smaller block size"
    );
}

#[test]
fn test_subsumed_group_is_dropped() {
    let files = parsed_files(&[
        ("__tests__/SubsumedA.java", JAVA),
        ("__tests__/SubsumedB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(
        result.groups.len(),
        1,
        "sub-runs of the one reported group must be dropped: {:?}",
        result.groups
    );
    assert_eq!(result.groups[0].locations.len(), 2);
    assert_eq!(result.groups[0].locations[0].source_lines, 15);
}

#[test]
fn test_partial_statement_run_is_found() {
    let files = parsed_files(&[("__tests__/PartialRun.java", JAVA)]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 1, "{:?}", result.groups);
    assert_eq!(result.groups[0].locations.len(), 2);
    // The shared run is 3 statements (alpha, beta, gamma), not the whole
    // 5-statement method body.
    for location in &result.groups[0].locations {
        let span = location.end_line - location.start_line + 1;
        assert!(
            span < 16,
            "the found run must be the inner 3-statement span, not the whole method: {span}"
        );
    }
}

#[test]
fn test_cross_language_blocks_never_group() {
    let files = parsed_files(&[
        ("__tests__/CrossLangJava.java", JAVA),
        ("__tests__/CrossLangJs.js", JS_TS),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(
        result.groups.len(),
        0,
        "identical token text across language families must never group: {:?}",
        result.groups
    );
}

/// D11 reused: a comment-only line inside a run does not count toward its
/// source-line total, so a run that is only ≥10 physical lines *because*
/// one of them is a comment must not qualify at `--min-clone-lines 10`
/// (review-ws3-r1-code.md row 5). The fixtures' shared run spans 10
/// physical lines with one comment-only line inside a call's argument
/// list, i.e. 9 D11 source lines.
#[test]
fn test_comment_only_lines_do_not_count_toward_min_clone_lines() {
    let files = parsed_files(&[
        ("__tests__/CommentInStatementA.java", JAVA),
        ("__tests__/CommentInStatementB.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert!(
        result.groups.is_empty(),
        "a comment-only line must not count toward --min-clone-lines: {:?}",
        result.groups
    );
}

#[test]
fn test_redundant_occurrences_excludes_the_canonically_first() {
    let files = parsed_files(&[
        ("__tests__/RankGX1.java", JAVA),
        ("__tests__/RankGX2.java", JAVA),
        ("__tests__/RankGX3.java", JAVA),
    ]);
    let result = clones::run(&files, 10);
    assert_eq!(result.groups.len(), 1, "{:?}", result.groups);
    let group = &result.groups[0];
    assert_eq!(group.locations.len(), 3);

    let redundant = redundant_occurrences(group);
    assert_eq!(redundant.len(), 2, "{redundant:?}");
    // The dropped location is the canonically first: the smallest
    // (path, start line).
    let first = &group.locations[0];
    assert!(!redundant
        .iter()
        .any(|location| location.relative_path == first.relative_path
            && location.start_line == first.start_line));

    let total: usize = redundant.iter().map(|location| location.source_lines).sum();
    assert_eq!(total, group.redundant_lines);
}

/// End-to-end: scanning the whole `tests/fixtures/clones` tree finds both
/// the Java and the JS/TS duplicate (D20's two families), ranks the Java
/// one first (12 redundant lines vs the JS/TS one's 10), and dropping
/// below 20 removes both (D17's `--min-clone-lines` is wired through).
#[test]
fn test_clone_scan_matches_hand_computation() {
    let root = fixture_root();
    let settings_at_ten = ScanSettings {
        output: std::env::temp_dir(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: 10,
    };
    let output = pipeline::run(root.to_str().expect("utf-8 fixture root"), settings_at_ten)
        .expect("pipeline run over the clones fixture tree");

    assert_eq!(output.clones.groups.len(), 2, "{:?}", output.clones.groups);
    let top = &output.clones.groups[0];
    assert_eq!(top.locations.len(), 2);
    assert_eq!(top.redundant_lines, 12, "{:?}", top.locations);
    let second = &output.clones.groups[1];
    assert_eq!(second.redundant_lines, 10, "{:?}", second.locations);

    let settings_at_twenty = ScanSettings {
        output: std::env::temp_dir(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: 20,
    };
    let output_twenty = pipeline::run(
        root.to_str().expect("utf-8 fixture root"),
        settings_at_twenty,
    )
    .expect("pipeline run over the clones fixture tree");
    assert_eq!(
        output_twenty.clones.groups.len(),
        0,
        "{:?}",
        output_twenty.clones.groups
    );
}

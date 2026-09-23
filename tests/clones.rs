use std::fs;
use std::path::{Path, PathBuf};

use nsd::clones::{self, redundant_occurrences};
use nsd::model::{DiscoveredFile, LanguageFamily, ScanSettings};
use nsd::parse::{self, ParsedFile};
use nsd::pipeline;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clones")
}

/// Parses every `(relative path, language)` pair under a given root.
fn parsed_files_under(root: &Path, paths: &[(&str, LanguageFamily)]) -> Vec<ParsedFile> {
    let files: Vec<DiscoveredFile> = paths
        .iter()
        .map(|(path, language)| DiscoveredFile {
            relative_path: PathBuf::from(*path),
            language: *language,
        })
        .collect();
    let (parsed, failures) = parse::parse_all(root, &files);
    assert!(
        failures.is_empty(),
        "unexpected parse failures: {failures:?}"
    );
    parsed
}

/// Parses every `(relative path, language)` pair under `tests/fixtures/clones/`.
fn parsed_files(paths: &[(&str, LanguageFamily)]) -> Vec<ParsedFile> {
    parsed_files_under(&fixture_root(), paths)
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

/// WS-3: proves the family prefix survived the hasher swap to BLAKE3
/// by forming an actual group per language, not merely absence of a
/// merged one (`test_cross_language_blocks_never_group` above already
/// covers absence). Each language here has an internal duplicate pair of
/// the same normalized token text `test_cross_language_blocks_never_group`
/// uses, so the digest's family prefix, not the digest input, is the only
/// thing that can keep the two languages' groups apart.
#[test]
fn test_family_prefix_separates_identical_token_streams() {
    let files = parsed_files(&[
        ("__tests__/FamilyPrefixJavaA.java", JAVA),
        ("__tests__/FamilyPrefixJavaB.java", JAVA),
        ("__tests__/FamilyPrefixJsA.js", JS_TS),
        ("__tests__/FamilyPrefixJsB.js", JS_TS),
    ]);
    let result = clones::run(&files, 2);
    assert_eq!(result.groups.len(), 2, "{:?}", result.groups);
    let java_groups: Vec<_> = result
        .groups
        .iter()
        .filter(|group| group.language == JAVA)
        .collect();
    let js_groups: Vec<_> = result
        .groups
        .iter()
        .filter(|group| group.language == JS_TS)
        .collect();
    assert_eq!(java_groups.len(), 1, "{:?}", result.groups);
    assert_eq!(js_groups.len(), 1, "{:?}", result.groups);
    assert_eq!(java_groups[0].locations.len(), 2, "{:?}", java_groups[0]);
    assert_eq!(js_groups[0].locations.len(), 2, "{:?}", js_groups[0]);
    for location in &java_groups[0].locations {
        assert!(location.relative_path.to_string_lossy().ends_with(".java"));
    }
    for location in &js_groups[0].locations {
        assert!(location.relative_path.to_string_lossy().ends_with(".js"));
    }
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

/// WS-4: the IR retarget's own floor. `--include-tests` over the *whole*
/// `tests/fixtures/clones` tree (every fixture the suite above exercises
/// individually, plus the two `src/`-rooted pairs `test_clone_scan_matches_
/// hand_computation` covers) exercises every container kind and every
/// per-test scenario at once; this pins the exact group set the pre-IR
/// implementation produced for that same scan (captured before the
/// retarget, at `min_clone_lines: 10`), so the retarget in `src/clones/
/// mod.rs` is checked against it rather than against a re-derivation of
/// the same numbers.
#[test]
fn test_ir_backed_groups_match_the_pre_ir_groups_on_every_fixture() {
    let root = fixture_root();
    let settings = ScanSettings {
        output: std::env::temp_dir(),
        include_tests: true,
        exclude: Vec::new(),
        min_clone_lines: 10,
    };
    let output = pipeline::run(root.to_str().expect("utf-8 fixture root"), settings)
        .expect("pipeline run over the whole clones fixture tree");

    // (path, start_line, end_line, source_lines) for one location.
    type ExpectedLocation<'a> = (&'a str, usize, usize, usize);
    // (redundant_lines, language, locations), in the exact canonical group
    // and location order `clones::run` produces.
    type ExpectedGroup<'a> = (usize, LanguageFamily, Vec<ExpectedLocation<'a>>);
    let expected: Vec<ExpectedGroup> = vec![
        (
            72,
            JAVA,
            vec![
                ("__tests__/DupIdenticalA.java", 3, 14, 12),
                ("__tests__/DupIdenticalB.java", 4, 18, 12),
                ("__tests__/IdentifierA.java", 3, 14, 12),
                ("__tests__/SubsumedA.java", 3, 14, 12),
                ("__tests__/SubsumedB.java", 3, 14, 12),
                ("src/main/java/DupA.java", 5, 16, 12),
                ("src/main/java/DupB.java", 6, 17, 12),
            ],
        ),
        (
            20,
            JAVA,
            vec![
                ("__tests__/RankGX1.java", 3, 12, 10),
                ("__tests__/RankGX2.java", 3, 12, 10),
                ("__tests__/RankGX3.java", 3, 12, 10),
            ],
        ),
        (
            20,
            JS_TS,
            vec![
                ("__tests__/SwitchDupJs.js", 4, 13, 10),
                ("__tests__/SwitchDupJs.js", 15, 24, 10),
                ("__tests__/SwitchDupJs.js", 26, 35, 10),
            ],
        ),
        (
            15,
            JAVA,
            vec![
                ("__tests__/RankGY1.java", 3, 17, 15),
                ("__tests__/RankGY2.java", 3, 17, 15),
            ],
        ),
        (
            15,
            JAVA,
            vec![
                ("__tests__/SubsumedA.java", 3, 17, 15),
                ("__tests__/SubsumedB.java", 3, 17, 15),
            ],
        ),
        (
            12,
            JAVA,
            vec![
                ("__tests__/ConstructorBodyA.java", 3, 14, 12),
                ("__tests__/ConstructorBodyB.java", 3, 14, 12),
            ],
        ),
        (
            12,
            JS_TS,
            vec![
                ("__tests__/ModuleLevelA.js", 1, 12, 12),
                ("__tests__/ModuleLevelB.js", 1, 12, 12),
            ],
        ),
        (
            12,
            JAVA,
            vec![
                ("__tests__/PartialRun.java", 4, 15, 12),
                ("__tests__/PartialRun.java", 21, 32, 12),
            ],
        ),
        (
            10,
            JAVA,
            vec![
                ("__tests__/SwitchDupJava.java", 5, 14, 10),
                ("__tests__/SwitchDupJava.java", 16, 25, 10),
            ],
        ),
        (
            10,
            JS_TS,
            vec![("src/widgetA.js", 2, 11, 10), ("src/widgetB.js", 2, 11, 10)],
        ),
    ];

    assert_eq!(
        output.clones.groups.len(),
        expected.len(),
        "{:?}",
        output.clones.groups
    );
    for (group, (redundant_lines, language, locations)) in
        output.clones.groups.iter().zip(expected.iter())
    {
        assert_eq!(group.redundant_lines, *redundant_lines, "{:?}", group);
        assert_eq!(group.language, *language, "{:?}", group);
        assert_eq!(group.locations.len(), locations.len(), "{:?}", group);
        for (location, (path, start_line, end_line, source_lines)) in
            group.locations.iter().zip(locations.iter())
        {
            assert_eq!(location.relative_path, PathBuf::from(*path), "{:?}", group);
            assert_eq!(location.start_line, *start_line, "{:?}", group);
            assert_eq!(location.end_line, *end_line, "{:?}", group);
            assert_eq!(location.source_lines, *source_lines, "{:?}", group);
        }
    }
}

/// WS-4: the container/statement enumeration is now `ir::IrNode::
/// is_clone_statement` membership, not a `(language, "<grammar kind>")`
/// match — this exercises all six D15 containers (Java `block`,
/// `constructor_body`, `switch_block_statement_group`; JS/TS
/// `statement_block`, `program`, `switch_case`/`switch_default` body) and
/// pins that each still yields the exact statement list (as the group it
/// forms) the pre-retarget container match produced.
#[test]
fn test_statement_children_come_from_ir_block_membership() {
    // Java `block` (a method body).
    let block_files = parsed_files(&[
        ("__tests__/DupIdenticalA.java", JAVA),
        ("__tests__/DupIdenticalB.java", JAVA),
    ]);
    let block_result = clones::run(&block_files, 10);
    assert_eq!(block_result.groups.len(), 1, "{:?}", block_result.groups);
    assert_eq!(block_result.groups[0].locations.len(), 2);

    // Java `constructor_body`.
    let constructor_files = parsed_files(&[
        ("__tests__/ConstructorBodyA.java", JAVA),
        ("__tests__/ConstructorBodyB.java", JAVA),
    ]);
    let constructor_result = clones::run(&constructor_files, 10);
    assert_eq!(
        constructor_result.groups.len(),
        1,
        "{:?}",
        constructor_result.groups
    );
    assert_eq!(constructor_result.groups[0].locations.len(), 2);

    // Java `switch_block_statement_group`.
    let java_switch_files = parsed_files(&[("__tests__/SwitchDupJava.java", JAVA)]);
    let java_switch_result = clones::run(&java_switch_files, 10);
    assert_eq!(
        java_switch_result.groups.len(),
        1,
        "{:?}",
        java_switch_result.groups
    );
    assert_eq!(java_switch_result.groups[0].locations.len(), 2);

    // JS/TS `statement_block` (a function body).
    let statement_block_files =
        parsed_files(&[("src/widgetA.js", JS_TS), ("src/widgetB.js", JS_TS)]);
    let statement_block_result = clones::run(&statement_block_files, 10);
    assert_eq!(
        statement_block_result.groups.len(),
        1,
        "{:?}",
        statement_block_result.groups
    );
    assert_eq!(statement_block_result.groups[0].locations.len(), 2);

    // JS/TS top-level `program`.
    let program_files = parsed_files(&[
        ("__tests__/ModuleLevelA.js", JS_TS),
        ("__tests__/ModuleLevelB.js", JS_TS),
    ]);
    let program_result = clones::run(&program_files, 10);
    assert_eq!(
        program_result.groups.len(),
        1,
        "{:?}",
        program_result.groups
    );
    assert_eq!(program_result.groups[0].locations.len(), 2);

    // JS/TS `switch_case`/`switch_default` body.
    let js_switch_files = parsed_files(&[("__tests__/SwitchDupJs.js", JS_TS)]);
    let js_switch_result = clones::run(&js_switch_files, 10);
    assert_eq!(
        js_switch_result.groups.len(),
        1,
        "{:?}",
        js_switch_result.groups
    );
    assert_eq!(js_switch_result.groups[0].locations.len(), 3);
}

/// WS-4: the IR token stream `ir_statement_tokens` derives from is built
/// fresh per `clones::run` call (no caching across calls), so its BLAKE3
/// group key must be deterministic across two independent runs over the
/// same input, not merely stable within one run.
#[test]
fn test_fingerprint_is_stable_across_two_runs_of_the_ir_path() {
    let files = parsed_files(&[
        ("__tests__/DupIdenticalA.java", JAVA),
        ("__tests__/DupIdenticalB.java", JAVA),
    ]);
    let first = clones::run(&files, 10);
    let second = clones::run(&files, 10);
    assert_eq!(first.groups.len(), 1, "{:?}", first.groups);
    assert_eq!(first.groups, second.groups);
}

/// triage-ws4-r2.md row 1: `tree-sitter-javascript` 0.25.0 aliases
/// `seq('static', /\s+/, 'get', /\s*\n/)` to the single anonymous kind
/// `"static get"` (`grammar.js:1252`) — its own source text carries
/// whatever internal whitespace and trailing newline the author wrote,
/// unlike most anonymous tokens whose text *is* their `kind()`. Two
/// copy-pasted blocks differing only in that token's internal spacing must
/// still fingerprint identically. Written into a `tempfile::tempdir()`
/// rather than under `tests/fixtures/clones/`, so this never retrips the
/// neutrality gate's corpus membership the way WS-3 round 1's parity
/// fixtures did.
#[test]
fn test_static_get_whitespace_does_not_break_the_group() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("StaticGetA.js"),
        "class Sample {\n  static get\n  value() {\n    return 1;\n  }\n}\n\nfunction helper() {\n  return 2;\n}\n",
    )
    .expect("write StaticGetA.js");
    fs::write(
        dir.path().join("StaticGetB.js"),
        "class Sample {\n  static  get\n  value() {\n    return 1;\n  }\n}\n\nfunction helper() {\n  return 2;\n}\n",
    )
    .expect("write StaticGetB.js");
    let files = parsed_files_under(
        dir.path(),
        &[("StaticGetA.js", JS_TS), ("StaticGetB.js", JS_TS)],
    );
    let result = clones::run(&files, 3);
    assert_eq!(
        result.groups.len(),
        1,
        "an anonymous leaf's internal whitespace must not change its normalized token: {:?}",
        result.groups
    );
    assert_eq!(result.groups[0].locations.len(), 2, "{:?}", result.groups);
}

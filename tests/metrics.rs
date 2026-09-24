use std::path::PathBuf;

use nsd::metrics;
use nsd::model::{Callable, DiscoveredFile, LanguageFamily, ParseFailureReason, ScanSettings};
use nsd::parse::{self, ParsedFile};
use nsd::pipeline;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metrics")
}

fn parse_fixture(relative_path: &str, language: LanguageFamily) -> ParsedFile {
    let root = fixture_root();
    let files = vec![DiscoveredFile {
        relative_path: PathBuf::from(relative_path),
        language,
    }];
    let (parsed, failures) = parse::parse_all(&root, &files);
    assert!(
        failures.is_empty(),
        "unexpected parse failures for {relative_path}: {failures:?}"
    );
    parsed
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected one parsed file for {relative_path}"))
}

fn callables_for(relative_path: &str, language: LanguageFamily) -> Vec<Callable> {
    let parsed = parse_fixture(relative_path, language);
    metrics::run(std::slice::from_ref(&parsed), false).callables
}

fn make_callable(cc: u32, sloc: usize) -> Callable {
    make_callable_named(cc, sloc, "File.java", 1)
}

fn make_callable_named(cc: u32, sloc: usize, path: &str, start_line: usize) -> Callable {
    Callable {
        relative_path: PathBuf::from(path),
        language: LanguageFamily::Java,
        name: "callable".to_string(),
        start_line,
        cc,
        sloc,
        mass: metrics::mass(cc, sloc),
    }
}

#[test]
fn test_java_cc_counts_documented_constructs() {
    let callables = callables_for("__tests__/Decisions.java", LanguageFamily::Java);
    assert_eq!(callables.len(), 1, "{callables:?}");
    let decide = &callables[0];
    assert_eq!(decide.name, "decide");
    assert_eq!(decide.cc, 15);
}

#[test]
fn test_jsts_cc_counts_documented_constructs() {
    let callables = callables_for("__tests__/JsDecisions.js", LanguageFamily::JsTs);
    assert_eq!(callables.len(), 1, "{callables:?}");
    let decide = &callables[0];
    assert_eq!(decide.name, "decide");
    // 1 (base) + if(1) + for(1) + for_in(1) + while(1) + do(1) + catch(1)
    // + ternary(1) + case(1) + case(1) + &&(1) + ||(1) + ??(1) = 13
    assert_eq!(decide.cc, 13);
}

#[test]
fn test_jsts_for_of_counts_once() {
    let callables = callables_for("__tests__/ForOfOptional.js", LanguageFamily::JsTs);
    assert_eq!(callables.len(), 1, "{callables:?}");
    let for_of = &callables[0];
    assert_eq!(for_of.name, "forOfOptional");
    // 1 (base) + 1 (for..of) + 0 (?.) + 1 (??) = 3
    assert_eq!(for_of.cc, 3);
}

#[test]
fn test_extension_grammar_mapping() {
    let fixtures = [
        "__tests__/Widget.tsx",
        "__tests__/Widget.jsx",
        "__tests__/mod.mjs",
        "__tests__/mod.cjs",
    ];
    let root = fixture_root();
    for relative_path in fixtures {
        let files = vec![DiscoveredFile {
            relative_path: PathBuf::from(relative_path),
            language: LanguageFamily::JsTs,
        }];
        let (parsed, failures) = parse::parse_all(&root, &files);
        assert!(
            failures.is_empty(),
            "{relative_path}: unexpected parse failures {failures:?}"
        );
        assert_eq!(parsed.len(), 1, "{relative_path}");
        let callables = metrics::run(&parsed, false).callables;
        assert_eq!(callables.len(), 1, "{relative_path}: {callables:?}");
    }
}

#[test]
fn test_switch_case_labels_count_each_default_excluded() {
    let callables = callables_for("__tests__/SwitchForms.java", LanguageFamily::Java);
    assert_eq!(callables.len(), 2, "{callables:?}");
    let colon = callables
        .iter()
        .find(|callable| callable.name == "colonForm")
        .expect("colonForm callable");
    let arrow = callables
        .iter()
        .find(|callable| callable.name == "arrowForm")
        .expect("arrowForm callable");
    assert_eq!(colon.cc, 3);
    assert_eq!(arrow.cc, 3);
}

#[test]
fn test_callable_kinds_with_and_without_body() {
    let java_callables = callables_for("__tests__/CallableKinds.java", LanguageFamily::Java);
    assert_eq!(java_callables.len(), 5, "{java_callables:?}");
    for excluded in ["area", "perimeter"] {
        assert!(
            !java_callables
                .iter()
                .any(|callable| callable.name == excluded),
            "bodyless {excluded} must not be a callable"
        );
    }

    let ts_callables = callables_for("__tests__/CallableKinds.ts", LanguageFamily::JsTs);
    assert_eq!(ts_callables.len(), 5, "{ts_callables:?}");
    for excluded in ["signatureOnly", "render"] {
        assert!(
            !ts_callables
                .iter()
                .any(|callable| callable.name == excluded),
            "bodyless {excluded} must not be a callable"
        );
    }
}

#[test]
fn test_nested_callable_not_double_counted() {
    let callables = callables_for("__tests__/NestedCallable.js", LanguageFamily::JsTs);
    assert_eq!(callables.len(), 2, "{callables:?}");
    let outer = callables
        .iter()
        .find(|callable| callable.name == "outer")
        .expect("outer callable");
    let inner = callables
        .iter()
        .find(|callable| callable.name == "inner")
        .expect("inner callable");

    assert_eq!(outer.cc, 2);
    assert_eq!(outer.sloc, 4);
    assert_eq!(outer.mass, 4.0);

    assert_eq!(inner.cc, 2);
    assert_eq!(inner.sloc, 1);
    assert_eq!(inner.mass, 2.0);

    let total_mass: f64 = callables.iter().map(|callable| callable.mass).sum();
    assert_eq!(total_mass, 6.0);
}

#[test]
fn test_nested_callable_excluded_as_body_and_as_first_child() {
    // D9 coverage: a nested callable can be its enclosing callable's `body`
    // field directly (no wrapping block — `curried`) or the first child of
    // a statement inside the enclosing block (`outer`). Both shapes must
    // exclude the nested callable's span from the enclosing callable's
    // cc/sloc, same as a later-sibling nested callable already does.
    let callables = callables_for(
        "__tests__/NestedExpressionCallable.js",
        LanguageFamily::JsTs,
    );
    assert_eq!(callables.len(), 4, "{callables:?}");

    let curried = callables
        .iter()
        .find(|callable| callable.name == "curried")
        .expect("curried callable");
    assert_eq!(curried.cc, 1);
    assert_eq!(curried.sloc, 0);
    assert_eq!(curried.mass, 0.0);

    let anon_at_1 = callables
        .iter()
        .find(|callable| callable.name == "<anonymous>@1")
        .expect("<anonymous>@1 callable");
    assert_eq!(anon_at_1.cc, 3);
    assert_eq!(anon_at_1.sloc, 1);

    let outer = callables
        .iter()
        .find(|callable| callable.name == "outer")
        .expect("outer callable");
    assert_eq!(outer.cc, 1);
    assert_eq!(outer.sloc, 0);

    let anon_at_4 = callables
        .iter()
        .find(|callable| callable.name == "<anonymous>@4")
        .expect("<anonymous>@4 callable");
    assert_eq!(anon_at_4.cc, 2);
    assert_eq!(anon_at_4.sloc, 1);
}

#[test]
fn test_callable_name_resolution() {
    let callables = callables_for("__tests__/NameResolution.js", LanguageFamily::JsTs);
    let names: Vec<String> = callables
        .iter()
        .map(|callable| callable.name.clone())
        .collect();
    assert!(names.contains(&"named".to_string()), "{names:?}");
    assert!(names.contains(&"f".to_string()), "{names:?}");
    assert!(names.contains(&"handler".to_string()), "{names:?}");
    assert!(
        names.iter().any(|name| name.starts_with("<anonymous>@")),
        "{names:?}"
    );
    assert!(names.contains(&"<anonymous>@13".to_string()), "{names:?}");
}

#[test]
fn test_sloc_excludes_comments_blank_and_delimiter_only_lines() {
    let callables = callables_for("__tests__/SlocLines.js", LanguageFamily::JsTs);
    assert_eq!(callables.len(), 1, "{callables:?}");
    assert_eq!(callables[0].sloc, 6);
}

#[test]
fn test_syntax_blocks_include_module_level_blocks() {
    let parsed = parse_fixture("__tests__/TopLevelBlock.js", LanguageFamily::JsTs);
    let result = metrics::run(std::slice::from_ref(&parsed), false);
    assert_eq!(result.syntax_blocks.len(), 2, "{:?}", result.syntax_blocks);

    let module_level_if = result
        .syntax_blocks
        .iter()
        .find(|block| block.start_line == 1)
        .expect("module-level if block at line 1");
    assert_eq!(module_level_if.end_line, 3);

    let function_body = result
        .syntax_blocks
        .iter()
        .find(|block| block.start_line == 5)
        .expect("plain()'s function body block at line 5");
    assert_eq!(function_body.end_line, 7);
}

/// How many levels deep the generated array literal nests for
/// `test_deeply_nested_file_does_not_abort_the_scan` -- deep enough to blow
/// a 2 MiB rayon worker stack under the old per-depth-level recursion.
const DEEP_NESTING_LEVELS: usize = 15_000;

#[test]
fn test_deeply_nested_file_does_not_abort_the_scan() {
    let dir = std::env::temp_dir().join(format!("nsd_ws2_deep_nest_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir for the deep-nesting fixture");

    let opening = "[".repeat(DEEP_NESTING_LEVELS);
    let closing = "]".repeat(DEEP_NESTING_LEVELS);
    let source = format!("function wrap() {{\n  const deep = {opening}{closing};\n}}\n");
    std::fs::write(dir.join("Deep.js"), source).expect("write the deep-nesting fixture");

    let settings = ScanSettings {
        output: std::env::temp_dir(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: 10,
    };
    let result = pipeline::run(dir.to_str().expect("utf-8 temp dir"), settings);

    let _ = std::fs::remove_dir_all(&dir);

    let output = result.expect("pipeline::run must not abort on a deeply nested file");
    assert_eq!(
        output.parse_failures.len(),
        0,
        "{:?}",
        output.parse_failures
    );
    assert_eq!(output.metrics.callables.len(), 1);
}

#[test]
fn test_file_scanned_lines_counts_the_whole_file() {
    let java = parse_fixture("erosion/HighComplexity.java", LanguageFamily::Java);
    let java_summaries = metrics::run(std::slice::from_ref(&java), false).file_scan_summaries;
    assert_eq!(java_summaries.len(), 1, "{java_summaries:?}");
    assert_eq!(
        java_summaries[0].relative_path,
        PathBuf::from("erosion/HighComplexity.java")
    );
    assert_eq!(java_summaries[0].scanned_lines, 11);

    let js = parse_fixture("erosion/LowComplexity.js", LanguageFamily::JsTs);
    let js_summaries = metrics::run(std::slice::from_ref(&js), false).file_scan_summaries;
    assert_eq!(js_summaries.len(), 1, "{js_summaries:?}");
    assert_eq!(
        js_summaries[0].relative_path,
        PathBuf::from("erosion/LowComplexity.js")
    );
    assert_eq!(js_summaries[0].scanned_lines, 10);
}

/// A bare `break;`/`continue;`/`return;` (no label or expression) must
/// still count toward SLOC: lines 2, 3, 4, 6, 7 and 10 of
/// `BareControlFlow.js` hold executable code (the `for`/`if` headers plus
/// the three bare statements); the delimiter-only `}` lines do not.
#[test]
fn test_bare_control_flow_statements_count_as_sloc() {
    let callables = callables_for("__tests__/BareControlFlow.js", LanguageFamily::JsTs);
    assert_eq!(callables.len(), 1, "{callables:?}");
    assert_eq!(callables[0].sloc, 6);
}

#[test]
fn test_mass_is_cc_times_sqrt_sloc() {
    assert_eq!(metrics::mass(4, 9), 12.0);
    assert_eq!(metrics::mass(1, 1), 1.0);
}

#[test]
fn test_erosion_is_mass_fraction_above_cc_10() {
    let not_eroded = make_callable(10, 4); // mass = 10 * 2 = 20.0, not eroded (cc not > 10)
    let eroded = make_callable(11, 9); // mass = 11 * 3 = 33.0, eroded (cc > 10)
    let callables = vec![not_eroded.clone(), eroded.clone()];
    let expected = eroded.mass / (not_eroded.mass + eroded.mass);
    assert_eq!(metrics::erosion(&callables), expected);
    assert_eq!(expected, 33.0 / 53.0);
}

#[test]
fn test_erosion_is_zero_when_no_callables() {
    assert_eq!(metrics::erosion(&[]), 0.0);
}

/// Edge case 1 (this test): a non-empty callable slice where none exceed the
/// CC erosion threshold. Expected `+0.0` (`is_sign_positive()`); actual
/// before the fix is `-0.0`, because `eroded_mass` sums an empty filtered
/// iterator and `Iterator::sum::<f64>()` over an empty sequence is `-0.0` on
/// this toolchain, and `-0.0 / total_mass` stays `-0.0`.
/// Edge case 2 (covered by `test_erosion_is_mass_fraction_above_cc_10` and
/// `test_erosion_scan_matches_hand_computation` above): all mass eroded, or
/// a genuine fraction — both unaffected by this fix, since `eroded_mass` is
/// non-zero there.
#[test]
fn test_erosion_with_no_eroded_mass_is_positive_zero() {
    let callables = vec![make_callable(1, 1), make_callable(2, 4)];
    let erosion = metrics::erosion(&callables);
    assert_eq!(erosion, 0.0);
    assert!(
        erosion.is_sign_positive(),
        "erosion with no eroded mass must be +0.0, got {erosion}"
    );
}

#[test]
fn test_top25_ranked_by_cc_desc_and_truncated() {
    let mut callables = Vec::new();
    for i in 0..30u32 {
        callables.push(make_callable_named(i, 1, &format!("file{i:02}.js"), 1));
    }
    let ranked = metrics::rank_top_callables(&callables);
    assert_eq!(ranked.len(), 25);
    for pair in ranked.windows(2) {
        assert!(pair[0].cc >= pair[1].cc);
    }
    assert_eq!(ranked[0].cc, 29);
    assert_eq!(ranked[24].cc, 5);

    let tie_a = make_callable_named(5, 1, "b.js", 2);
    let tie_b = make_callable_named(5, 1, "a.js", 1);
    let ranked_ties = metrics::rank_top_callables(&[tie_a.clone(), tie_b.clone()]);
    assert_eq!(ranked_ties[0].relative_path, tie_b.relative_path);
    assert_eq!(ranked_ties[1].relative_path, tie_a.relative_path);
}

/// `docs/cc-rules.md` walks through this exact hand computation: the
/// single `erosion/HighComplexity.java` callable is `CC=12, SLOC=9,
/// mass=36.0`; together with `erosion/LowComplexity.js`'s `CC=8, SLOC=9,
/// mass=24.0`, total mass is `60.0`, so erosion is `36.0 / 60.0 == 0.6`.
/// `broken/Broken.ts` sits in the same fixture tree and fails to parse
/// (D18), so this same scan also carries `incomplete == true`.
#[test]
fn test_erosion_scan_matches_hand_computation() {
    let high = callables_for("erosion/HighComplexity.java", LanguageFamily::Java);
    assert_eq!(high.len(), 1, "{high:?}");
    assert_eq!(high[0].cc, 12);
    assert_eq!(high[0].sloc, 9);
    assert_eq!(high[0].mass, 36.0);

    let low = callables_for("erosion/LowComplexity.js", LanguageFamily::JsTs);
    assert_eq!(low.len(), 1, "{low:?}");
    assert_eq!(low[0].cc, 8);
    assert_eq!(low[0].sloc, 9);
    assert_eq!(low[0].mass, 24.0);

    let settings = ScanSettings {
        output: std::env::temp_dir(),
        include_tests: false,
        exclude: Vec::new(),
        min_clone_lines: 10,
    };
    let root = fixture_root();
    let output = pipeline::run(root.to_str().expect("utf-8 fixture root"), settings)
        .expect("pipeline run over the fixture tree");

    assert_eq!(output.metrics.erosion, 0.6);
    assert!(
        output.metrics.incomplete,
        "broken/Broken.ts fails to parse, so this scan must be marked incomplete"
    );
    assert_eq!(output.parse_failures.len(), 1);
    assert_eq!(
        output.parse_failures[0].relative_path,
        PathBuf::from("broken/Broken.ts")
    );

    let top = &output.metrics.top25[0];
    assert_eq!(
        top.relative_path,
        PathBuf::from("erosion/HighComplexity.java")
    );
    assert_eq!(top.cc, 12);
    assert_eq!(top.sloc, 9);
    assert_eq!(top.mass, 36.0);
    assert_eq!(top.start_line, 2);
}

#[test]
fn test_unparseable_file_is_skipped_and_marks_incomplete() {
    let root = fixture_root();
    let files = vec![DiscoveredFile {
        relative_path: PathBuf::from("broken/Broken.ts"),
        language: LanguageFamily::JsTs,
    }];
    let (parsed, failures) = parse::parse_all(&root, &files);
    // WS-6 declared delta: salvage (`nsd-plan-final.md`'s salvage row) means
    // a syntax-error file still produces a `ParsedFile` alongside its
    // `ParseFailure` -- it is no longer dropped wholesale, only its damaged
    // entities are excluded downstream. `failures` still carries the
    // `SyntaxError` entry (unchanged below), so `incomplete` still flips.
    assert_eq!(
        parsed.len(),
        1,
        "salvage still hands the file to later stages"
    );
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].relative_path, PathBuf::from("broken/Broken.ts"));
    assert_eq!(failures[0].reason, ParseFailureReason::SyntaxError);

    let result = metrics::run(&parsed, !failures.is_empty());
    assert!(result.incomplete);
}

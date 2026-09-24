use std::path::PathBuf;

use nsd::ir::{self, IrNode};
use nsd::lower;
use nsd::model::{DiscoveredFile, LanguageFamily};
use nsd::parse::{self, ParsedFile};
use nsd::rules::{self, ALL_RULE_IDS};

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

/// Builds a `ParsedFile` directly from an inline source string, bypassing
/// on-disk discovery — used by the three new IR-retarget regression tests
/// below so they don't grow `tests/fixtures/rules/` (and, with it,
/// `tests/neutrality.rs`'s clean-corpus membership, which is WS-2's fence,
/// not this stream's).
fn parse_inline_java(source: &str) -> ParsedFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_java_orchard::LANGUAGE.into())
        .expect("java grammar");
    let tree = parser.parse(source, None).expect("java parse");
    ParsedFile {
        relative_path: PathBuf::from("Inline.java"),
        language: JAVA,
        source: source.to_string(),
        tree,
    }
}

fn parse_inline_jsts(source: &str) -> ParsedFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .expect("javascript grammar");
    let tree = parser.parse(source, None).expect("javascript parse");
    ParsedFile {
        relative_path: PathBuf::from("inline.js"),
        language: JS_TS,
        source: source.to_string(),
        tree,
    }
}

/// Every `IrNode` in `root`'s tree, pre-order (parent before its children) —
/// mirrors `tests/ir_lowering.rs`'s own `collect` helper.
fn collect<'a>(root: &'a IrNode, out: &mut Vec<&'a IrNode>) {
    out.push(root);
    for child in &root.children {
        collect(child, out);
    }
}

/// The first (pre-order, so outermost) IR node whose span starts on
/// `line` (1-based) — used to locate a specific statement without a
/// grammar-kind string, since the IR carries none.
fn node_starting_at_line(root: &IrNode, line: u32) -> &IrNode {
    let mut nodes = Vec::new();
    collect(root, &mut nodes);
    nodes
        .into_iter()
        .find(|node| node.span.start_line == line)
        .unwrap_or_else(|| panic!("no IR node starts at line {line}"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules")
}

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

#[test]
fn test_each_java_rule_fires_once_on_its_fixture() {
    let files = parsed_files(&[("__tests__/JavaRulesFixture.java", JAVA)]);
    let findings = rules::find_findings(&files);

    // Exact (start_line, end_line, flagged_lines) per rule id, hand-counted
    // against __tests__/JavaRulesFixture.java: dropping the D11 filter, or
    // moving a finding to the wrong line, must fail one of these.
    let expected: [(&'static str, usize, usize, Vec<usize>); 3] = [
        (rules::JAVA_UNREACHABLE_AFTER_RETURN, 8, 9, vec![8, 9]),
        (rules::JAVA_EMPTY_CATCH, 15, 16, vec![15]),
        (rules::JAVA_REDUNDANT_ELSE_AFTER_RETURN, 22, 24, vec![23]),
    ];
    assert_eq!(findings.len(), expected.len(), "{findings:?}");
    for (rule_id, start_line, end_line, flagged_lines) in expected {
        let hits: Vec<_> = findings.iter().filter(|f| f.rule_id == rule_id).collect();
        assert_eq!(
            hits.len(),
            1,
            "{rule_id} should fire exactly once: {findings:?}"
        );
        let hit = hits[0];
        assert_eq!(hit.language, LanguageFamily::Java);
        assert!(hit.start_line > 0);
        assert!(hit.end_line >= hit.start_line);
        assert_eq!(
            hit.relative_path,
            PathBuf::from("__tests__/JavaRulesFixture.java")
        );
        assert_eq!(hit.start_line, start_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    }
}

#[test]
fn test_each_jsts_rule_fires_once_on_its_fixture() {
    let files = parsed_files(&[("__tests__/JsRulesFixture.js", JS_TS)]);
    let findings = rules::find_findings(&files);

    // Exact (start_line, end_line, flagged_lines) per rule id, hand-counted
    // against __tests__/JsRulesFixture.js.
    let expected: [(&'static str, usize, usize, Vec<usize>); 3] = [
        (rules::JSTS_UNREACHABLE_AFTER_RETURN, 4, 5, vec![4, 5]),
        (rules::JSTS_EMPTY_CATCH, 11, 12, vec![11]),
        (rules::JSTS_REDUNDANT_ELSE_AFTER_RETURN, 18, 20, vec![19]),
    ];
    // JSTS_UNREACHABLE_AFTER_RETURN's second hit: `mixedExemptAndDead` mixes
    // an exempt hoisted `helper` declaration (line 27) with two genuinely
    // dead statements (lines 28-29). This pins the exemption's scope — it
    // drops only the exempt statements from the finding, not the whole
    // finding whenever any exempt statement is present; an over-broad
    // mutant doing the latter drops this hit entirely and fails the
    // assertions below.
    let second_unreachable_hit: (&'static str, usize, usize, Vec<usize>) =
        (rules::JSTS_UNREACHABLE_AFTER_RETURN, 28, 29, vec![28, 29]);

    assert_eq!(findings.len(), expected.len() + 1, "{findings:?}");
    for (rule_id, start_line, end_line, flagged_lines) in expected {
        let hits: Vec<_> = findings.iter().filter(|f| f.rule_id == rule_id).collect();
        let expected_hit_count = if rule_id == rules::JSTS_UNREACHABLE_AFTER_RETURN {
            2
        } else {
            1
        };
        assert_eq!(
            hits.len(),
            expected_hit_count,
            "{rule_id} should fire {expected_hit_count} time(s): {findings:?}"
        );
        let hit = hits[0];
        assert_eq!(hit.language, LanguageFamily::JsTs);
        assert_eq!(hit.start_line, start_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
        assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    }

    let (rule_id, start_line, end_line, flagged_lines) = second_unreachable_hit;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == rule_id && f.start_line == start_line)
        .unwrap_or_else(|| {
            panic!("expected a second {rule_id} hit at line {start_line}: {findings:?}")
        });
    assert_eq!(hit.language, LanguageFamily::JsTs);
    assert_eq!(hit.end_line, end_line, "{rule_id}: {hit:?}");
    assert_eq!(hit.flagged_lines, flagged_lines, "{rule_id}: {hit:?}");
    assert!(
        !hit.flagged_lines.contains(&27),
        "the second unreachable-after-return hit must exclude the exempt helper declaration's line: {hit:?}"
    );
}

/// D22 IR retarget: one predicate, `ir::is_terminator`, answers
/// terminator-ness for a Java `return`/`break`/`throw` and its JS/TS
/// counterpart alike — no `language ==` branch inside the predicate
/// itself (`ir::is_terminator`'s own body only matches on `terminator`,
/// never on `LanguageFamily`). Each terminator statement is located by the
/// line it starts on, not by a grammar-kind string, since the IR carries
/// none; a non-terminator statement (each fixture's own method/function
/// header line) must answer `false` in both families too, so the
/// predicate isn't merely "always true here".
#[test]
fn test_terminator_predicate_is_language_agnostic() {
    let java_source = "class C {\n    int m(int a) {\n        if (a > 0) {\n            return 1;\n        }\n        for (int i = 0; i < a; i++) {\n            if (i == 2) {\n                break;\n            }\n        }\n        throw new RuntimeException();\n    }\n}\n";
    let java_ir = lower::lower_file(&parse_inline_java(java_source)).root;
    assert!(ir::is_terminator(node_starting_at_line(&java_ir, 4))); // return 1;
    assert!(ir::is_terminator(node_starting_at_line(&java_ir, 8))); // break;
    assert!(ir::is_terminator(node_starting_at_line(&java_ir, 11))); // throw ...;
    assert!(!ir::is_terminator(node_starting_at_line(&java_ir, 2))); // int m(int a) {

    let js_source = "function m(a) {\n    if (a > 0) {\n        return 1;\n    }\n    for (let i = 0; i < a; i++) {\n        if (i === 2) {\n            break;\n        }\n    }\n    throw new Error();\n}\n";
    let js_ir = lower::lower_file(&parse_inline_jsts(js_source)).root;
    assert!(ir::is_terminator(node_starting_at_line(&js_ir, 3))); // return 1;
    assert!(ir::is_terminator(node_starting_at_line(&js_ir, 7))); // break;
    assert!(ir::is_terminator(node_starting_at_line(&js_ir, 10))); // throw ...;
    assert!(!ir::is_terminator(node_starting_at_line(&js_ir, 1))); // function m(a) {
}

/// D22 IR retarget: `find_unreachable_after_return` keys on the narrower
/// `ir::is_unreachable_terminator` (`Return`/`Throw`/`Break`), not the wider
/// `ir::is_terminator` (which also matches `continue`) — the pre-IR rule's
/// own `UNREACHABLE_TERMINATOR_KINDS` excluded `continue_statement` too
/// (`687a86b:src/rules/mod.rs:57-58`), so a loop body's `continue;` must not
/// make the statement after it "unreachable".
#[test]
fn test_continue_in_a_loop_does_not_trigger_unreachable_after_return() {
    let source = "class C {\n    void m() {\n        for (int i = 0; i < 3; i++) {\n            continue;\n            doThing();\n        }\n    }\n}\n";
    let files = vec![parse_inline_java(source)];
    let findings = rules::find_findings(&files);
    assert!(
        !findings
            .iter()
            .any(|f| f.rule_id == rules::JAVA_UNREACHABLE_AFTER_RETURN),
        "continue must not be treated as an unreachable-after-return terminator: {findings:?}"
    );
}

/// D22 IR retarget: `always_returns` keys on the narrower
/// `ir::is_return_or_throw` (`Return`/`Throw`), not the wider
/// `ir::is_terminator` (which also matches `break`) — the pre-IR rule's own
/// `always_returns` matched only `return_statement`/`throw_statement` too
/// (`687a86b:src/rules/mod.rs:386-394`), so an `if`'s consequence ending in a
/// bare `break;` inside a loop must not make its sibling `else` "redundant".
#[test]
fn test_break_terminated_consequence_does_not_trigger_redundant_else() {
    let source = "class C {\n    void m() {\n        for (int i = 0; i < 3; i++) {\n            if (i == 2) {\n                break;\n            } else {\n                doThing();\n            }\n        }\n    }\n}\n";
    let files = vec![parse_inline_java(source)];
    let findings = rules::find_findings(&files);
    assert!(
        !findings
            .iter()
            .any(|f| f.rule_id == rules::JAVA_REDUNDANT_ELSE_AFTER_RETURN),
        "a break-terminated consequence must not make its else 'redundant': {findings:?}"
    );
}

/// D22 IR retarget: `is_unreachable_container`'s `is_root` disjunct is the
/// one case block-membership alone cannot see — JS/TS's top-level `program`
/// node is never itself a `{ }` block. A program-level `throw` must still
/// make the statements after it fire `*-UNREACHABLE-AFTER-RETURN`, exactly
/// as it did pre-retarget when `"program"` was named explicitly in the
/// grammar-string container list.
#[test]
fn test_unreachable_after_return_fires_at_the_jsts_program_top_level() {
    let source = "throw new Error(\"boom\");\nconsole.log(\"a\");\nconsole.log(\"b\");\n";
    let files = vec![parse_inline_jsts(source)];
    let findings = rules::find_findings(&files);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let hit = &findings[0];
    assert_eq!(hit.rule_id, rules::JSTS_UNREACHABLE_AFTER_RETURN);
    assert_eq!(hit.start_line, 2, "{hit:?}");
    assert_eq!(hit.end_line, 3, "{hit:?}");
    assert_eq!(hit.flagged_lines, vec![2, 3], "{hit:?}");
}

/// D22 IR retarget: `*-EMPTY-CATCH` reads `IrNode::in_catch_body` (the IR's
/// own catch-body-membership flag), not a `catch_clause` grammar string —
/// a genuinely empty catch still fires, and a catch holding only a comment
/// still declines, exactly as it did pre-retarget. A third case, in both
/// families: a non-empty catch nested inside another catch's body.
/// `IrNode::in_catch_body` is transitive (true for the body block itself
/// *and* every node inside it), so a naive "first child with
/// `in_catch_body`" search on the *inner* `catch` clause finds the
/// anonymous `catch` keyword token (which inherits `in_catch_body` from the
/// outer body it sits in, and has no children of its own) before it finds
/// the inner clause's actual body block — misreporting a genuinely
/// non-empty inner catch as empty.
#[test]
fn test_empty_catch_is_detected_through_ir_catch_body_membership() {
    let java_source = "class C {\n    void m() {\n        try {\n            doThing();\n        }\n        catch (Exception e) {\n        }\n        try {\n            doThing();\n        }\n        catch (Exception e) {\n            // ignored\n        }\n        try {\n            doThing();\n        }\n        catch (Exception e) {\n            try {\n                doOther();\n            }\n            catch (Exception e2) {\n                doAnother();\n            }\n        }\n    }\n}\n";
    let files = vec![parse_inline_java(java_source)];
    let findings = rules::find_findings(&files);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let hit = &findings[0];
    assert_eq!(hit.rule_id, rules::JAVA_EMPTY_CATCH);
    assert_eq!(hit.start_line, 6, "{hit:?}");
    assert_eq!(hit.end_line, 7, "{hit:?}");
    assert_eq!(hit.flagged_lines, vec![6], "{hit:?}");
    assert!(
        !findings.iter().any(|f| f.start_line == 21),
        "a non-empty catch nested inside another catch's body must not be misreported as empty: {findings:?}"
    );

    let js_source = "function m() {\n    try {\n        doThing();\n    }\n    catch (e) {\n    }\n    try {\n        doThing();\n    }\n    catch (e) {\n        // ignored\n    }\n    try {\n        doThing();\n    }\n    catch (e) {\n        try {\n            doOther();\n        }\n        catch (e2) {\n            doAnother();\n        }\n    }\n}\n";
    let jsts_files = vec![parse_inline_jsts(js_source)];
    let jsts_findings = rules::find_findings(&jsts_files);
    assert_eq!(jsts_findings.len(), 1, "{jsts_findings:?}");
    let jsts_hit = &jsts_findings[0];
    assert_eq!(jsts_hit.rule_id, rules::JSTS_EMPTY_CATCH);
    assert_eq!(jsts_hit.start_line, 5, "{jsts_hit:?}");
    assert_eq!(jsts_hit.end_line, 6, "{jsts_hit:?}");
    assert_eq!(jsts_hit.flagged_lines, vec![5], "{jsts_hit:?}");
    assert!(
        !jsts_findings.iter().any(|f| f.start_line == 20),
        "a non-empty catch nested inside another catch's body must not be misreported as empty: {jsts_findings:?}"
    );
}

/// D22 IR retarget: the documented JS/TS-only exemption
/// (`IrNode::is_hoisted_or_type_only`) survives the retarget and does not
/// leak into Java — this is a plain regression test for the retarget, not
/// a clearing of the `LanguageFamily::JsTs` guard row at
/// `src/rules/mod.rs:339` (that stays open, deferred to M0c). A JS/TS
/// hoisted function declaration after a `return` is exempt and produces no
/// finding; the same shape in Java (whose lowering hardcodes
/// `is_hoisted_or_type_only: false` for every kind) has no such exemption
/// and must still fire.
#[test]
fn test_hoisted_and_type_only_exemption_stays_jsts_only() {
    let java_source =
        "class C {\n    void m() {\n        return;\n        class Local {\n        }\n    }\n}\n";
    let js_source = "function m() {\n    return;\n    function helper() {\n    }\n}\n";
    let files = vec![parse_inline_java(java_source), parse_inline_jsts(js_source)];
    let findings = rules::find_findings(&files);

    assert_eq!(findings.len(), 1, "{findings:?}");
    let hit = &findings[0];
    assert_eq!(hit.rule_id, rules::JAVA_UNREACHABLE_AFTER_RETURN, "{hit:?}");
    assert_eq!(hit.language, LanguageFamily::Java);
    assert_eq!(hit.start_line, 4, "{hit:?}");
    assert_eq!(hit.end_line, 5, "{hit:?}");
    assert_eq!(hit.flagged_lines, vec![4], "{hit:?}");
    assert!(
        !findings
            .iter()
            .any(|finding| finding.rule_id == rules::JSTS_UNREACHABLE_AFTER_RETURN),
        "the JS/TS hoisted-function exemption must still hold: {findings:?}"
    );
}

#[test]
fn test_clean_fixture_produces_no_findings() {
    let files = parsed_files(&[
        ("__tests__/CleanJava.java", JAVA),
        ("__tests__/CleanJs.js", JS_TS),
        ("__tests__/CleanTs.ts", JS_TS),
    ]);
    let findings = rules::find_findings(&files);
    assert!(findings.is_empty(), "{findings:?}");
}

/// Every `JAVA-*`/`JSTS-*` token in `text`, treating any character that is
/// not an uppercase ASCII letter or `-` as a separator — so a token
/// embedded in backticks, headings or prose (e.g. `` `JAVA-EMPTY-CATCH` ``)
/// is extracted whole and nothing else is mistaken for a rule id.
fn extract_rule_id_like_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut token_start: Option<usize> = None;
    let mut push_if_rule_id = |start: usize, end: usize| {
        let token = &text[start..end];
        if token.starts_with("JAVA-") || token.starts_with("JSTS-") {
            tokens.push(token);
        }
    };
    for (index, ch) in text.char_indices() {
        let is_token_char = ch.is_ascii_uppercase() || ch == '-';
        match (is_token_char, token_start) {
            (true, None) => token_start = Some(index),
            (false, Some(start)) => {
                push_if_rule_id(start, index);
                token_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = token_start {
        push_if_rule_id(start, text.len());
    }
    tokens
}

#[test]
fn test_every_rule_id_is_documented() {
    let docs_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/wasteful-rules.md");
    let docs = std::fs::read_to_string(&docs_path).expect("docs/wasteful-rules.md must exist");
    for rule_id in ALL_RULE_IDS {
        assert!(
            docs.contains(rule_id),
            "docs/wasteful-rules.md must document {rule_id}"
        );
    }
    // Reverse direction: a documented-but-unimplemented or typo'd id would
    // pass the forward loop above (`docs.contains` only checks the six real
    // ids are present) but must fail here.
    for token in extract_rule_id_like_tokens(&docs) {
        assert!(
            ALL_RULE_IDS.contains(&token),
            "docs/wasteful-rules.md mentions {token}, which is not in ALL_RULE_IDS {ALL_RULE_IDS:?}"
        );
    }
}

//! Integration tests for M0b's IR core and both lowerings
//! (`nsd-plan-final.md` M0b items 3-5). `src/lower/mod.rs`'s own in-crate
//! tests already prove the clone-token floor and the D11 `executable`
//! predicate across every in-repo fixture; this file exercises the rest of
//! *What the IR must carry*: spans, decision kinds, the three structural
//! predicates, typed damage, and the version constants.

use std::path::{Path, PathBuf};

use nsd::ir::{self, DamageKind, DecisionKind, IrNode, TerminatorKind};
use nsd::lower;
use nsd::metrics;
use nsd::model::{DiscoveredFile, Grammar, LanguageFamily};
use nsd::parse::{self, ParsedFile};

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ir")
}

fn metrics_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metrics")
}

fn clones_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clones")
}

fn rules_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules")
}

/// Parses every `(relative path, language)` pair under `root` via the
/// production `parse::parse_all` path -- the same helper shape
/// `tests/clones.rs` uses. Every fixture passed here must parse without
/// error; the three damage fixtures use `parse_damaged` below instead,
/// since `parse_all` itself routes an `ERROR`-tree file to a `ParseFailure`
/// rather than a `ParsedFile` (the same filter that keeps a damaged file
/// out of the real pipeline's clones/metrics/rules stages).
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

/// `parsed_files_under`, rooted at `tests/fixtures/ir/` -- the root every
/// pre-round-3 test in this file already assumes.
fn parsed_files(paths: &[(&str, LanguageFamily)]) -> Vec<ParsedFile> {
    parsed_files_under(&fixture_root(), paths)
}

/// Parses one fixture file directly with tree-sitter, bypassing
/// `parse::parse_all`'s `has_error` rejection: the three damage fixtures
/// are deliberately unparseable-without-error, and the lowering must still
/// classify their `ERROR` nodes -- production never calls the lowering on
/// such a file today (M0b item 5 wires the lowering in without retargeting
/// any analyzer), but the lowering itself must not assume an error-free
/// tree.
fn parse_damaged(path: &str, language: LanguageFamily) -> ParsedFile {
    let full_path = fixture_root().join(path);
    let source = std::fs::read_to_string(&full_path)
        .unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
    let extension = full_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let grammar = Grammar::for_extension(extension)
        .unwrap_or_else(|| panic!("no grammar for extension {extension:?}"));
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language(grammar))
        .expect("set_language");
    let tree = parser.parse(&source, None).expect("parse");
    ParsedFile {
        relative_path: PathBuf::from(path),
        language,
        source,
        tree,
    }
}

/// `parse_damaged`, but for an inline source string rather than a fixture
/// file on disk: `throw` has no fixture under `tests/fixtures/` to read (see
/// `test_throw_is_a_return_or_throw_terminator` below), and adding one would
/// move the neutrality baseline (see *Scope*'s no-new-fixture rule) -- an
/// inline source adds no file under `tests/fixtures/`.
fn parse_inline(source: &str, grammar: Grammar, language: LanguageFamily) -> ParsedFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language(grammar))
        .expect("set_language");
    let tree = parser.parse(source, None).expect("parse");
    ParsedFile {
        relative_path: PathBuf::from("inline"),
        language,
        source: source.to_string(),
        tree,
    }
}

fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
    match grammar {
        Grammar::Java => tree_sitter_java_orchard::LANGUAGE.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

/// Every `IrNode` in `root`'s tree, pre-order.
fn collect<'a>(root: &'a IrNode, out: &mut Vec<&'a IrNode>) {
    out.push(root);
    for child in &root.children {
        collect(child, out);
    }
}

/// Every `DecisionKind` in `root`'s tree, pre-order.
fn decision_sequence(root: &IrNode) -> Vec<DecisionKind> {
    let mut nodes = Vec::new();
    collect(root, &mut nodes);
    nodes.into_iter().filter_map(|node| node.decision).collect()
}

#[test]
fn test_version_constants_are_exported() {
    // M0c-9: `IR_VERSION`/`JAVA_LOWERING_VERSION` bumped 2 -> 3 (removing
    // `DamageKind::JavaVarargsAnnotation` changes what the Java lowering
    // carries); `JSTS_LOWERING_VERSION` is untouched by the grammar swap.
    // M1-7: `IR_VERSION` bumped 3 -> 4 -- `IrCallable` gains `kind`/
    // `is_anonymous`/`signature`/`owner_chain`, changing what a downstream
    // consumer (WS-1's own `identity` module) reads off it. Both lowering
    // versions are unchanged: neither lowering's own classification of a
    // node changed, only what `IrCallable` additionally records about one.
    // M3-4: `IR_VERSION` bumped 4 -> 5 -- `IrFile` gains `excluded_callables`,
    // which A101 reads off the lowering.
    assert_eq!(nsd::ir::IR_VERSION, 5);
    assert_eq!(lower::JAVA_LOWERING_VERSION, 3);
    assert_eq!(lower::JSTS_LOWERING_VERSION, 2);
}

/// Iterative pre-order traversal via a single reused `TreeCursor`: the same
/// shape as `crate::exec_lines::for_each_descendant`, a fresh copy
/// here since that one is `pub(crate)`.
fn for_each_descendant<'tree>(
    root: tree_sitter::Node<'tree>,
    mut visit: impl FnMut(tree_sitter::Node<'tree>),
) {
    let mut cursor = root.walk();
    loop {
        visit(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

#[test]
fn test_every_ir_node_carries_a_byte_and_line_span() {
    let files = parsed_files(&[("Decisions.java", JAVA)]);
    let ir_file = lower::lower_file(&files[0]);

    let mut nodes = Vec::new();
    collect(&ir_file.root, &mut nodes);
    assert!(!nodes.is_empty());

    for node in &nodes {
        assert!(node.span.start_byte <= node.span.end_byte);
        assert!(node.span.start_line >= 1);
        assert!(node.span.end_line >= node.span.start_line);
    }

    // Per-node parity against the tree-sitter tree the IR was built from,
    // not just internal self-consistency: `build_ir` visits the same
    // TreeCursor shape in the same pre-order, so `nodes[i]` and
    // `ts_nodes[i]` name the same source node, one node per the lowering's
    // 1:1 correspondence.
    let mut ts_nodes = Vec::new();
    for_each_descendant(files[0].tree.root_node(), |node| ts_nodes.push(node));
    assert_eq!(nodes.len(), ts_nodes.len());
    for (ir_node, ts_node) in nodes.iter().zip(ts_nodes.iter()) {
        assert_eq!(ir_node.span.start_byte, ts_node.start_byte() as u32);
        assert_eq!(ir_node.span.end_byte, ts_node.end_byte() as u32);
        assert_eq!(
            ir_node.span.start_line,
            ts_node.start_position().row as u32 + 1
        );
        assert_eq!(ir_node.span.end_line, ts_node.end_position().row as u32 + 1);
    }

    let root_span = ir_file.root.span;
    assert_eq!(root_span.start_byte, 0);
    assert_eq!(root_span.end_byte, files[0].source.len() as u32);
    assert_eq!(root_span.start_line, 1);
}

#[test]
fn test_decision_kinds_are_language_agnostic() {
    let expected = vec![
        DecisionKind::Branch,
        DecisionKind::Loop,
        DecisionKind::Loop,
        DecisionKind::Case,
        DecisionKind::Catch,
        DecisionKind::Ternary,
        DecisionKind::And,
        DecisionKind::Or,
    ];

    let java_files = parsed_files(&[("Decisions.java", JAVA)]);
    let java_ir = lower::lower_file(&java_files[0]);
    assert_eq!(decision_sequence(&java_ir.root), expected);

    let ts_files = parsed_files(&[("decisions.ts", JS_TS)]);
    let ts_ir = lower::lower_file(&ts_files[0]);
    assert_eq!(decision_sequence(&ts_ir.root), expected);
}

#[test]
fn test_structural_predicates_answer_terminator_block_membership_and_catch_body() {
    for (path, language) in [
        ("StructuralPredicates.java", JAVA),
        ("structural_predicates.ts", JS_TS),
    ] {
        let files = parsed_files(&[(path, language)]);
        let ir_file = lower::lower_file(&files[0]);

        let mut nodes = Vec::new();
        collect(&ir_file.root, &mut nodes);

        let terminators: Vec<&&IrNode> = nodes
            .iter()
            .filter(|node| ir::is_terminator(node))
            .collect();
        assert_eq!(terminators.len(), 2, "{path}: {terminators:#?}");

        let in_catch: Vec<_> = terminators
            .iter()
            .filter(|node| ir::is_in_catch_body(node))
            .collect();
        assert_eq!(
            in_catch.len(),
            1,
            "{path}: expected exactly one catch-body terminator"
        );

        let outside_catch: Vec<_> = terminators
            .iter()
            .filter(|node| !ir::is_in_catch_body(node))
            .collect();
        assert_eq!(
            outside_catch.len(),
            1,
            "{path}: expected exactly one non-catch-body terminator"
        );
        assert!(
            ir::is_block_member(outside_catch[0]),
            "{path}: the plain `return` sits directly in the method's block"
        );

        // Negative block-membership case: a `catch_clause` node itself (the
        // one IrNode with `decision == Some(DecisionKind::Catch)`) sits
        // directly inside a `try_statement`, never a block, so it must read
        // as not-a-block-member -- and neither does the file root, which has
        // no parent at all.
        let catch_clause = nodes
            .iter()
            .find(|node| node.decision == Some(DecisionKind::Catch))
            .expect("a catch_clause decision node");
        assert!(
            !ir::is_block_member(catch_clause),
            "{path}: a catch_clause's own parent is a try_statement, not a block"
        );
        assert!(
            !ir::is_block_member(&ir_file.root),
            "{path}: the file root has no parent"
        );
    }
}

/// M0c-9 (Decision 4): `DamageKind::JavaVarargsAnnotation` is removed --
/// orchard's grammar no longer produces the `ERROR` shape it named (see
/// `test_java_varargs_annotation_parses_clean_under_orchard` below) -- so
/// only the two JS/TS damage classes stay typed here. Renamed from
/// `test_damage_spans_are_typed_for_all_three_known_classes`.
#[test]
fn test_damage_spans_are_typed_for_both_known_classes() {
    let ts_using = parse_damaged("TsUsingParameter.ts", JS_TS);
    let ts_ir = lower::lower_file(&ts_using);
    assert!(
        ts_ir
            .damage
            .iter()
            .any(|damage| damage.kind == DamageKind::TsUsingParameterName),
        "{:#?}",
        ts_ir.damage
    );

    let jsx_entity = parse_damaged("JsxUnterminatedEntity.tsx", JS_TS);
    let jsx_ir = lower::lower_file(&jsx_entity);
    assert!(
        jsx_ir
            .damage
            .iter()
            .any(|damage| damage.kind == DamageKind::JsxUnterminatedEntity),
        "{:#?}",
        jsx_ir.damage
    );
}

/// M0c-9/M0c-10's gate, at the unit level: `JavaVarargsAnnotation.java`'s
/// `Class<?> @Nullable ... cs` parameter, which produced an `ERROR` node
/// under `tree-sitter-java` 0.23.5, must parse with **no** syntax error at
/// all under orchard -- `parsed_files` (not `parse_damaged`) is deliberate
/// here: it goes through the production `parse::parse_all` path and asserts
/// zero `ParseFailure`s, so this fails loudly pre-swap rather than silently
/// tolerating a residual error.
#[test]
fn test_java_varargs_annotation_parses_clean_under_orchard() {
    let files = parsed_files(&[("JavaVarargsAnnotation.java", JAVA)]);
    let ir_file = lower::lower_file(&files[0]);
    assert!(ir_file.damage.is_empty(), "{:#?}", ir_file.damage);
}

#[test]
fn test_clean_source_lowers_with_no_damage_spans() {
    let files = parsed_files(&[("Decisions.java", JAVA)]);
    let ir_file = lower::lower_file(&files[0]);
    assert!(ir_file.damage.is_empty(), "{:#?}", ir_file.damage);

    let files = parsed_files(&[("decisions.ts", JS_TS)]);
    let ir_file = lower::lower_file(&files[0]);
    assert!(ir_file.damage.is_empty(), "{:#?}", ir_file.damage);
}

// test_ir_tokens_differ_when_a_statement_differs moved to
// src/lower/mod.rs::tests (Decision 11): it now calls
// `statement_token_stream` directly rather than reading `IrNode::token`,
// which carried no reader in `src/` and was removed.

/// WS-3's blocker (`implementer-ws3-r1.md`): the pre-IR `&&`/`||`/`??` weight
/// table had one shared arm; the lowering's own `binary_expression` match
/// dropped the `??` arm. `??` now lowers to `DecisionKind::Or`, not a new
/// variant (`metrics::decision_weight`'s exhaustive match has no wildcard
/// arm), and `?.` (optional chaining) still lowers to no decision at all.
#[test]
fn test_nullish_coalescing_is_a_decision_point() {
    let files = parsed_files_under(
        &metrics_fixture_root(),
        &[("__tests__/ForOfOptional.js", JS_TS)],
    );
    let ir_file = lower::lower_file(&files[0]);

    let mut nodes = Vec::new();
    collect(&ir_file.root, &mut nodes);

    // Line 7: `const fallback = maybe ?? 0;`
    let line7_or: Vec<_> = nodes
        .iter()
        .filter(|node| node.span.start_line == 7 && node.decision == Some(DecisionKind::Or))
        .collect();
    assert_eq!(line7_or.len(), 1, "{line7_or:#?}");

    // Line 6: `const value = maybe?.value;` -- `?.` is not a decision point.
    let line6_decisions: Vec<_> = nodes
        .iter()
        .filter(|node| node.span.start_line == 6 && node.decision.is_some())
        .collect();
    assert!(line6_decisions.is_empty(), "{line6_decisions:#?}");
}

/// D8 + D10 parity with the live analyzer (`implementer-ws3-r1.md`'s other
/// blocker), proved before WS-3 swaps to reading the IR: `IrFile::callables`'
/// `(name, start_line)` pairs, concatenated across the three fixtures in
/// path order, equal `metrics::run`'s own `MetricsResult::callables` pairs
/// (globally sorted by path then start line, which is the same order for
/// these three paths). Also pins the one Java callable kind with no `body`
/// field: `static_initializer`'s body span comes from its unnamed `block`
/// child, not the declaration's own span.
#[test]
fn test_callable_table_matches_the_metrics_callables() {
    let root = metrics_fixture_root();
    let files = parsed_files_under(
        &root,
        &[
            ("__tests__/CallableKinds.java", JAVA),
            ("__tests__/CallableKinds.ts", JS_TS),
            ("__tests__/NameResolution.js", JS_TS),
        ],
    );

    let expected: Vec<(String, usize)> = metrics::run(&files, false)
        .callables
        .iter()
        .map(|callable| (callable.name.clone(), callable.start_line))
        .collect();

    let mut actual: Vec<(String, usize)> = Vec::new();
    let mut static_initializer: Option<(ir::Span, ir::Span, String)> = None;
    let mut regular: Option<(ir::Span, ir::Span, String)> = None;
    for file in &files {
        let ir_file = lower::lower_file(file);
        for callable in &ir_file.callables {
            actual.push((callable.name.clone(), callable.span.start_line as usize));
            if file.relative_path == Path::new("__tests__/CallableKinds.java")
                && callable.span.start_line == 18
            {
                static_initializer = Some((callable.span, callable.body_span, file.source.clone()));
            }
            if file.relative_path == Path::new("__tests__/CallableKinds.ts")
                && callable.span.start_line == 1
            {
                regular = Some((callable.span, callable.body_span, file.source.clone()));
            }
        }
    }

    assert_eq!(actual, expected);

    let (callable_span, body_span, source) =
        static_initializer.expect("the static_initializer callable");
    assert_eq!(body_span.start_line, 18, "{body_span:?}");
    assert_eq!(body_span.end_line, 20, "{body_span:?}");
    // Discriminating: the `static_initializer` declaration node also spans
    // lines 18-20 (`static { ... }`), so the line-only asserts above cannot
    // separate the body span from the declaration span -- the byte span can.
    assert!(
        body_span.start_byte > callable_span.start_byte,
        "{body_span:?} vs {callable_span:?}"
    );
    assert_eq!(
        &source[body_span.start_byte as usize..=body_span.start_byte as usize],
        "{",
        "{body_span:?}"
    );

    let (regular_span, regular_body_span, regular_source) =
        regular.expect("the CallableKinds.ts regular callable");
    assert!(
        regular_body_span.start_byte > regular_span.start_byte,
        "{regular_body_span:?} vs {regular_span:?}"
    );
    assert_eq!(
        &regular_source
            [regular_body_span.start_byte as usize..=regular_body_span.start_byte as usize],
        "{",
        "{regular_body_span:?}"
    );
}

/// `SyntaxBlock`'s three values, reproducible from the IR, in order:
/// `IrFile::blocks`' `(kind, start_line, end_line)` triples, concatenated
/// across the two fixtures in file order, equal `metrics::run`'s own
/// `MetricsResult::syntax_blocks` triples (never globally sorted, unlike
/// `callables` -- see `metrics::run`'s own doc comment).
#[test]
fn test_block_table_matches_the_metrics_syntax_blocks() {
    let mut files = parsed_files_under(
        &metrics_fixture_root(),
        &[
            ("__tests__/TopLevelBlock.js", JS_TS),
            ("__tests__/CallableKinds.java", JAVA),
        ],
    );
    files.extend(parsed_files(&[("StructuralPredicates.java", JAVA)]));

    let expected: Vec<(&'static str, usize, usize)> = metrics::run(&files, false)
        .syntax_blocks
        .iter()
        .map(|block| (block.kind, block.start_line, block.end_line))
        .collect();

    let mut actual: Vec<(&'static str, usize, usize)> = Vec::new();
    for file in &files {
        let ir_file = lower::lower_file(file);
        for block in &ir_file.blocks {
            actual.push((
                block.kind,
                block.span.start_line as usize,
                block.span.end_line as usize,
            ));
        }
    }

    assert_eq!(actual, expected);
    assert!(!actual.is_empty());
}

/// WS-4's blocker (`implementer-ws4-r1.md`): a comment and an anonymous
/// punctuation leaf both read `executable == false` today, indistinguishable
/// from each other. `is_comment` + `is_named` separate them: a comment is
/// `is_comment && is_named`; an anonymous leaf like `;`/`(`/`)` is neither.
#[test]
fn test_comment_and_named_markers_separate_a_comment_from_anonymous_punctuation() {
    let files = parsed_files_under(
        &clones_fixture_root(),
        &[("__tests__/CommentInStatementA.java", JAVA)],
    );
    let ir_file = lower::lower_file(&files[0]);

    let mut nodes = Vec::new();
    collect(&ir_file.root, &mut nodes);
    let mut ts_nodes = Vec::new();
    for_each_descendant(files[0].tree.root_node(), |node| ts_nodes.push(node));
    assert_eq!(nodes.len(), ts_nodes.len());

    let comment_index = ts_nodes
        .iter()
        .position(|node| node.start_position().row + 1 == 5 && node.kind() == "line_comment")
        .expect("a line_comment on line 5");
    let comment = nodes[comment_index];
    assert!(comment.is_comment, "{comment:#?}");
    assert!(comment.is_named, "{comment:#?}");
    assert!(!comment.executable, "{comment:#?}");

    let mut punctuation_checked = 0usize;
    for (index, ts_node) in ts_nodes.iter().enumerate() {
        if matches!(ts_node.kind(), ";" | "(" | ")") {
            let leaf = nodes[index];
            assert!(!leaf.is_comment, "{leaf:#?}");
            assert!(!leaf.is_named, "{leaf:#?}");
            assert!(!leaf.executable, "{leaf:#?}");
            punctuation_checked += 1;
        }
    }
    assert!(
        punctuation_checked > 0,
        "expected at least one `;`/`(`/`)` leaf"
    );
}

/// Round 2's discriminating-container proof (`test_clone_candidate_containers_
/// cover_every_container_arm`, `src/lower/mod.rs::tests`), reproduced through
/// `IrNode::is_clone_statement` -- the field this round promotes the same
/// classification into, rather than the tree-sitter-derived
/// `is_clone_statement` dispatcher that test calls directly.
#[test]
fn test_clone_statement_flag_reproduces_the_proven_container_set() {
    let java_files = parsed_files(&[("ContainerSet.java", JAVA)]);
    let java_ir = lower::lower_file(&java_files[0]);
    let mut java_nodes = Vec::new();
    collect(&java_ir.root, &mut java_nodes);
    let java_lines: Vec<u32> = java_nodes
        .iter()
        .filter(|node| node.is_clone_statement)
        .map(|node| node.span.start_line)
        .collect();
    assert_eq!(java_lines, vec![3, 4, 8, 10, 11], "{java_lines:?}");

    let ts_files = parsed_files(&[("container_set.ts", JS_TS)]);
    let ts_ir = lower::lower_file(&ts_files[0]);
    let mut ts_nodes = Vec::new();
    collect(&ts_ir.root, &mut ts_nodes);
    let ts_lines: Vec<u32> = ts_nodes
        .iter()
        .filter(|node| node.is_clone_statement)
        .map(|node| node.span.start_line)
        .collect();
    assert_eq!(ts_lines, vec![1, 2, 4, 5, 7, 8], "{ts_lines:?}");
}

/// WS-5's blocker (`implementer-ws5-r1.md`): the hoisted/type-only exemption
/// is JS/TS-only -- the Java lowering never sets it.
#[test]
fn test_hoisted_and_type_only_flag_is_jsts_only() {
    let root = rules_fixture_root();

    let ts_files = parsed_files_under(&root, &[("__tests__/CleanTs.ts", JS_TS)]);
    let ts_ir = lower::lower_file(&ts_files[0]);
    let mut ts_nodes = Vec::new();
    collect(&ts_ir.root, &mut ts_nodes);
    let ts_flagged_lines: Vec<u32> = ts_nodes
        .iter()
        .filter(|node| node.is_hoisted_or_type_only)
        .map(|node| node.span.start_line)
        .collect();
    // Exact set, not just "contains" (which an over-broad flag -- e.g. every
    // JS/TS node flagged -- would also satisfy): line 1 and 9's own
    // `function` declarations, line 4's `interface Unused`, line 12's `type
    // UnusedAlias`.
    assert_eq!(ts_flagged_lines, vec![1, 4, 9, 12], "{ts_flagged_lines:?}");

    let js_files = parsed_files_under(&root, &[("__tests__/JsRulesFixture.js", JS_TS)]);
    let js_ir = lower::lower_file(&js_files[0]);
    let mut js_nodes = Vec::new();
    collect(&js_ir.root, &mut js_nodes);
    let js_flagged_lines: Vec<u32> = js_nodes
        .iter()
        .filter(|node| node.is_hoisted_or_type_only)
        .map(|node| node.span.start_line)
        .collect();
    // Exact set: the seven `function` declarations, pre-order.
    assert_eq!(
        js_flagged_lines,
        vec![1, 8, 15, 23, 25, 27, 30],
        "{js_flagged_lines:?}"
    );

    let cj_files = parsed_files_under(&root, &[("__tests__/CleanJs.js", JS_TS)]);
    let cj_ir = lower::lower_file(&cj_files[0]);
    let mut cj_nodes = Vec::new();
    collect(&cj_ir.root, &mut cj_nodes);
    let cj_lines: Vec<u32> = cj_nodes
        .iter()
        .filter(|node| node.is_hoisted_or_type_only)
        .map(|node| node.span.start_line)
        .collect();
    // Exact set: the eight `function` declarations at 1/8/16/24/33/35/38/43
    // plus the `function*` at 46 (38 nests in 35, 46 nests in 43) -- this is
    // the only fixture in this test with a `generator_function_declaration`,
    // so it is the only block that would catch a dropped
    // `"generator_function_declaration"` arm in
    // `is_hoisted_or_type_only` (src/lower/jsts.rs).
    assert_eq!(
        cj_lines,
        vec![1, 8, 16, 24, 33, 35, 38, 43, 46],
        "{cj_lines:?}"
    );

    for (path, language) in [
        ("__tests__/JavaRulesFixture.java", JAVA),
        ("__tests__/CleanJava.java", JAVA),
    ] {
        let files = parsed_files_under(&root, &[(path, language)]);
        let ir_file = lower::lower_file(&files[0]);
        let mut nodes = Vec::new();
        collect(&ir_file.root, &mut nodes);
        assert!(
            nodes.iter().all(|node| !node.is_hoisted_or_type_only),
            "{path}: expected no node flagged"
        );
    }
}

/// WS-5's other blocker: the two narrower terminator subsets split `break`
/// from `continue` (only `break` is `is_unreachable_terminator`), and split
/// `return`/`throw` from `break`/`continue` (only `return`/`throw` is
/// `is_return_or_throw`).
#[test]
fn test_terminator_subsets_split_break_and_continue() {
    let files = parsed_files_under(
        &metrics_fixture_root(),
        &[("__tests__/BareControlFlow.js", JS_TS)],
    );
    let ir_file = lower::lower_file(&files[0]);
    let mut nodes = Vec::new();
    collect(&ir_file.root, &mut nodes);

    let continue_node = nodes
        .iter()
        .find(|node| node.span.start_line == 4 && ir::is_terminator(node))
        .expect("the continue on line 4");
    assert!(!ir::is_unreachable_terminator(continue_node));
    assert!(!ir::is_return_or_throw(continue_node));

    let break_node = nodes
        .iter()
        .find(|node| node.span.start_line == 7 && ir::is_terminator(node))
        .expect("the break on line 7");
    assert!(ir::is_unreachable_terminator(break_node));
    assert!(!ir::is_return_or_throw(break_node));

    let java_files = parsed_files(&[("StructuralPredicates.java", JAVA)]);
    let java_ir = lower::lower_file(&java_files[0]);
    let mut java_nodes = Vec::new();
    collect(&java_ir.root, &mut java_nodes);
    let returns: Vec<_> = java_nodes
        .iter()
        .filter(|node| {
            ir::is_terminator(node) && (node.span.start_line == 7 || node.span.start_line == 9)
        })
        .collect();
    assert_eq!(returns.len(), 2, "{returns:#?}");
    for ret in &returns {
        assert!(ir::is_unreachable_terminator(ret));
        assert!(ir::is_return_or_throw(ret));
    }
}

/// Round 2's HIGH-4 shrink (88 -> 48 bytes) must survive this round's four
/// new bool fields plus the `bool` -> `Option<TerminatorKind>` widening: a
/// scan retains one `IrNode` per tree-sitter node, so per-node size is the
/// dominant memory cost.
#[test]
fn test_ir_node_stays_within_56_bytes() {
    let size = std::mem::size_of::<IrNode>();
    assert!(size <= 56, "{size}");
}

/// `TerminatorKind::Throw` (D22/D7): no fixture under `tests/fixtures/`
/// contains a `throw` at all, so both lowerings' `"throw_statement" =>
/// Some(TerminatorKind::Throw)` arm is otherwise unasserted -- deleting it
/// from either lowering would leave the whole suite green. Uses
/// `parse_inline` rather than a new fixture file (see that helper's doc
/// comment).
#[test]
fn test_throw_is_a_return_or_throw_terminator() {
    let java_source = "class T { void m() { throw new RuntimeException(); } }";
    let java_file = parse_inline(java_source, Grammar::Java, JAVA);
    let java_ir = lower::lower_file(&java_file);
    let mut java_nodes = Vec::new();
    collect(&java_ir.root, &mut java_nodes);
    let java_throw = java_nodes
        .iter()
        .find(|node| node.terminator == Some(TerminatorKind::Throw))
        .expect("a throw_statement node");
    assert!(ir::is_terminator(java_throw));
    assert!(ir::is_unreachable_terminator(java_throw));
    assert!(ir::is_return_or_throw(java_throw));

    let js_source = "function m() { throw new Error(); }";
    let js_file = parse_inline(js_source, Grammar::JavaScript, JS_TS);
    let js_ir = lower::lower_file(&js_file);
    let mut js_nodes = Vec::new();
    collect(&js_ir.root, &mut js_nodes);
    let js_throw = js_nodes
        .iter()
        .find(|node| node.terminator == Some(TerminatorKind::Throw))
        .expect("a throw_statement node");
    assert!(ir::is_terminator(js_throw));
    assert!(ir::is_unreachable_terminator(js_throw));
    assert!(ir::is_return_or_throw(js_throw));
}

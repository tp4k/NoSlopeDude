//! Integration tests for M0b's IR core and both lowerings
//! (`nsd-plan-final.md` M0b items 3-5). `src/lower/mod.rs`'s own in-crate
//! tests already prove the clone-token floor and the D11 `executable`
//! predicate across every in-repo fixture; this file exercises the rest of
//! *What the IR must carry*: spans, decision kinds, the three structural
//! predicates, typed damage, and the version constants.

use std::path::PathBuf;

use nsd::ir::{self, DamageKind, DecisionKind, IrNode};
use nsd::lower;
use nsd::model::{DiscoveredFile, Grammar, LanguageFamily};
use nsd::parse::{self, ParsedFile};

const JAVA: LanguageFamily = LanguageFamily::Java;
const JS_TS: LanguageFamily = LanguageFamily::JsTs;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ir")
}

/// Parses every `(relative path, language)` pair under `tests/fixtures/ir/`
/// via the production `parse::parse_all` path -- the same helper shape
/// `tests/clones.rs` uses. Every fixture passed here must parse without
/// error; the three damage fixtures use `parse_damaged` below instead,
/// since `parse_all` itself routes an `ERROR`-tree file to a `ParseFailure`
/// rather than a `ParsedFile` (the same filter that keeps a damaged file
/// out of the real pipeline's clones/metrics/rules stages).
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

fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
    match grammar {
        Grammar::Java => tree_sitter_java::LANGUAGE.into(),
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
    assert_eq!(nsd::ir::IR_VERSION, 1);
    assert_eq!(lower::JAVA_LOWERING_VERSION, 1);
    assert_eq!(lower::JSTS_LOWERING_VERSION, 1);
}

/// Iterative pre-order traversal via a single reused `TreeCursor`: the same
/// shape as `src/lower/mod.rs:148-164`'s `for_each_descendant`, a fresh copy
/// here since that one is private to the `lower` module.
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

#[test]
fn test_damage_spans_are_typed_for_all_three_known_classes() {
    let java_varargs = parse_damaged("JavaVarargsAnnotation.java", JAVA);
    let java_ir = lower::lower_file(&java_varargs);
    assert!(
        java_ir
            .damage
            .iter()
            .any(|damage| damage.kind == DamageKind::JavaVarargsAnnotation),
        "{:#?}",
        java_ir.damage
    );

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

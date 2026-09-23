//! Integration tests for M0b's IR core and both lowerings
//! (`nsd-plan-final.md` M0b items 3-5). `src/lower/mod.rs`'s own in-crate
//! tests already prove the clone-token floor and the D11 `executable`
//! predicate across every in-repo fixture; this file exercises the rest of
//! *What the IR must carry*: spans, decision kinds, the three structural
//! predicates, typed damage, and the version constants.

use std::path::PathBuf;

use nsd::ir::{DamageKind, DecisionKind, IrNode};
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

    let root_span = ir_file.root.span;
    assert_eq!(root_span.start_byte, 0);
    assert_eq!(root_span.end_byte, files[0].source.len());
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

        let terminators: Vec<&&IrNode> = nodes.iter().filter(|node| node.is_terminator).collect();
        assert_eq!(terminators.len(), 2, "{path}: {terminators:#?}");

        let in_catch: Vec<_> = terminators
            .iter()
            .filter(|node| node.in_catch_body)
            .collect();
        assert_eq!(
            in_catch.len(),
            1,
            "{path}: expected exactly one catch-body terminator"
        );

        let outside_catch: Vec<_> = terminators
            .iter()
            .filter(|node| !node.in_catch_body)
            .collect();
        assert_eq!(
            outside_catch.len(),
            1,
            "{path}: expected exactly one non-catch-body terminator"
        );
        assert!(
            outside_catch[0].in_block,
            "{path}: the plain `return` sits directly in the method's block"
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

#[test]
fn test_ir_tokens_differ_when_a_statement_differs() {
    let files = parsed_files(&[("DifferA.java", JAVA), ("DifferB.java", JAVA)]);
    let a_ir = lower::lower_file(&files[0]);
    let b_ir = lower::lower_file(&files[1]);

    let mut a_nodes = Vec::new();
    collect(&a_ir.root, &mut a_nodes);
    let mut b_nodes = Vec::new();
    collect(&b_ir.root, &mut b_nodes);

    let a_tokens: Vec<&str> = a_nodes
        .iter()
        .filter_map(|node| node.token.as_deref())
        .collect();
    let b_tokens: Vec<&str> = b_nodes
        .iter()
        .filter_map(|node| node.token.as_deref())
        .collect();

    assert_eq!(a_tokens.len(), 1, "{a_tokens:?}");
    assert_eq!(b_tokens.len(), 1, "{b_tokens:?}");
    assert_ne!(a_tokens[0], b_tokens[0]);
}

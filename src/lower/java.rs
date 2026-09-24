//! The Java lowering: every `node.kind()` match this stream produces for a
//! Java file, isolated from `src/lower/jsts.rs`'s own table per the spec's
//! "no per-grammar strings on the IR itself" requirement -- only this file
//! and `src/lower/mod.rs` (via `Classification`) ever read a Java grammar
//! string.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{DamageKind, DecisionKind, Span, TerminatorKind};
use crate::model::LanguageFamily;

use super::{CallableInfo, Classification};

/// D22/D7's terminator kinds: identical node-kind literals to JS/TS's own
/// table, kept as a separate copy since each lowering owns its table
/// independently.
fn terminator_kind(kind: &str) -> Option<TerminatorKind> {
    match kind {
        "return_statement" => Some(TerminatorKind::Return),
        "break_statement" => Some(TerminatorKind::Break),
        "continue_statement" => Some(TerminatorKind::Continue),
        "throw_statement" => Some(TerminatorKind::Throw),
        _ => None,
    }
}

/// D8's Java callable kinds (`metrics::JAVA_CALLABLE_KINDS`, private to a
/// module this stream may not touch -- re-derived here rather than shared,
/// same as `is_block_kind` and `TERMINATOR_KINDS` already were).
const CALLABLE_KINDS: &[&str] = &[
    "method_declaration",
    "constructor_declaration",
    "compact_constructor_declaration",
    "static_initializer",
    "lambda_expression",
];

/// D8's body-node finder: every callable kind exposes it through the `body`
/// field, except `static_initializer`, whose direct `block` child carries no
/// field name.
fn callable_body(node: Node) -> Option<Node> {
    node.child_by_field_name("body")
        .or_else(|| first_child_of_kind(node, "block"))
}

fn first_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|child| child.kind() == kind);
    found
}

/// D10: the node's own `name` field; else the name from an enclosing
/// `variable_declarator`, `pair` or `assignment_expression`; else
/// `<anonymous>@<line>`. The same table as JS/TS's own copy -- see
/// `IrCallable`'s doc comment on why both lowerings carry it.
fn resolve_name(node: Node, parent: Option<Node>, source: &str) -> String {
    if let Some(name_node) = node.child_by_field_name("name") {
        return node_text(name_node, source);
    }
    if let Some(parent) = parent {
        let field = match parent.kind() {
            "variable_declarator" => Some("name"),
            "pair" => Some("key"),
            "assignment_expression" => Some("left"),
            _ => None,
        };
        if let Some(name_node) = field.and_then(|field| parent.child_by_field_name(field)) {
            return node_text(name_node, source);
        }
    }
    format!("<anonymous>@{}", node.start_position().row + 1)
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

fn callable_info(
    node: Node,
    kind: &str,
    parent: Option<Node>,
    source: &str,
) -> Option<CallableInfo> {
    if !CALLABLE_KINDS.contains(&kind) {
        return None;
    }
    let body = callable_body(node)?;
    Some(CallableInfo {
        body_span: Span::from_node(body),
        name: resolve_name(node, parent, source),
    })
}

/// Mirrors `metrics::is_block_kind`'s Java arm (private to `src/metrics/
/// mod.rs`): a `{ … }` scope is a `block` or a constructor's
/// `constructor_body`. Returns the matched arm's own literal rather than a
/// bool: `Node::kind()` in tree-sitter 0.27 borrows from `node`'s own
/// lifetime rather than promising `'static`, but `IrBlock::kind` is
/// `&'static str` (`model::SyntaxBlock::kind` must not move), so the caller
/// needs the match arm's `'static` literal, not `node.kind()` itself.
fn is_block_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "block" => Some("block"),
        "constructor_body" => Some("constructor_body"),
        _ => None,
    }
}

/// Java's non-default `switch_label`: `case`, never `default`. Mirrors
/// `metrics::is_default_label`'s own check (a `switch_label`'s first child's
/// kind is literally `"default"`).
fn is_default_label(node: Node) -> bool {
    node.child(0).is_some_and(|child| child.kind() == "default")
}

fn decision_kind(node: Node, kind: &str) -> Option<DecisionKind> {
    match kind {
        "if_statement" => Some(DecisionKind::Branch),
        "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement" => {
            Some(DecisionKind::Loop)
        }
        "switch_label" if !is_default_label(node) => Some(DecisionKind::Case),
        "catch_clause" => Some(DecisionKind::Catch),
        "ternary_expression" => Some(DecisionKind::Ternary),
        "binary_expression" => match operator_text(node) {
            Some("&&") => Some(DecisionKind::And),
            Some("||") => Some(DecisionKind::Or),
            _ => None,
        },
        _ => None,
    }
}

/// A `binary_expression`'s own operator token text, via the `operator`
/// field both grammars expose it under.
fn operator_text<'tree>(node: Node<'tree>) -> Option<&'tree str> {
    node.child_by_field_name("operator")
        .map(|child| child.kind())
}

/// Whether `node` itself is the block directly forming a `catch` clause's
/// body -- an O(1) check; `src/lower/mod.rs`'s `build_ir` combines this with
/// the parent's own already-computed flag to answer "or sits inside it"
/// without walking back up the tree per node. `parent_kind` is the caller's
/// already-threaded parent (`build_ir` passes it down the traversal instead
/// of calling `node.parent()`, which in tree-sitter 0.25.10 restarts at the
/// tree root and descends, turning one linear tree build into `Θ(depth)`
/// work per node).
fn is_catch_body_root(kind: &str, parent_kind: Option<&str>) -> bool {
    is_block_kind(kind).is_some() && parent_kind == Some("catch_clause")
}

/// D15's Java clone-candidate containers (`clones::statement_children`'s
/// Java arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `block`/`constructor_body`, or a
/// `switch_block_statement_group`'s direct child other than its own
/// `switch_label`. Takes the already-threaded `parent_kind` rather than
/// calling `node.parent()` -- see `is_catch_body_root`'s doc comment -- and
/// the caller's own already-computed `is_named`/`is_comment` (see
/// `classify`'s own doc comment: each is computed exactly once per node and
/// threaded into every helper, rather than re-derived here). Production: its
/// result is `IrNode::is_clone_statement`.
pub(super) fn is_clone_statement(
    kind: &str,
    parent_kind: Option<&str>,
    is_named: bool,
    is_comment: bool,
) -> bool {
    if !is_named || is_comment {
        return false;
    }
    match parent_kind {
        Some("block") | Some("constructor_body") => true,
        Some("switch_block_statement_group") => kind != "switch_label",
        _ => false,
    }
}

/// M0c-9: under orchard's grammar, a varargs parameter's annotation (e.g.
/// `void m(Class<?> @Nullable ... cs)`) no longer produces an `ERROR` node
/// directly inside `formal_parameters` -- `DamageKind::JavaVarargsAnnotation`
/// is removed (the shape it named is gone), so every remaining Java
/// `ERROR`/`MISSING` node falls back to `Unclassified` rather than a
/// dedicated class.
fn classify_damage(node: Node) -> Option<DamageKind> {
    if node.is_error() || node.is_missing() {
        return Some(DamageKind::Unclassified);
    }
    None
}

/// `parent` is the tree-sitter `Node` `src/lower/mod.rs`'s `build_ir` already
/// holds for this node's parent (threaded down the traversal in a stack
/// mirroring its own node stack), so nothing below this point calls
/// `node.parent()`. `kind`/`parent_kind`/`is_named`/`is_comment` are each
/// computed exactly once here and threaded into every helper, rather than
/// every helper re-deriving `node.kind()` (a strlen + full-UTF8-validate
/// call), `node.is_named()` or `is_comment_kind` independently.
pub(super) fn classify(node: Node, source: &str, parent: Option<Node>) -> Classification {
    let kind = node.kind();
    let parent_kind = parent.map(|parent| parent.kind());
    let is_named = node.is_named();
    let is_comment = is_comment_kind(kind, LanguageFamily::Java);
    Classification {
        decision: decision_kind(node, kind),
        terminator: terminator_kind(kind),
        in_block: parent_kind.is_some_and(|kind| is_block_kind(kind).is_some()),
        is_catch_body_root: is_catch_body_root(kind, parent_kind),
        damage: classify_damage(node),
        is_clone_statement: is_clone_statement(kind, parent_kind, is_named, is_comment),
        is_hoisted_or_type_only: false,
        block_kind: is_block_kind(kind),
        callable: callable_info(node, kind, parent, source),
        is_comment,
        is_named,
    }
}

//! The Java lowering: every `node.kind()` match this stream produces for a
//! Java file, isolated from `src/lower/jsts.rs`'s own table per the spec's
//! "no per-grammar strings on the IR itself" requirement -- only this file
//! and `src/lower/mod.rs` (via `Classification`) ever read a Java grammar
//! string.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{DamageKind, DecisionKind};
use crate::model::LanguageFamily;

use super::Classification;

/// D22/D7's terminator kinds: identical node-kind literals to JS/TS's own
/// table, kept as separate copies since each lowering owns its table
/// independently.
const TERMINATOR_KINDS: &[&str] = &[
    "return_statement",
    "break_statement",
    "continue_statement",
    "throw_statement",
];

/// Mirrors `metrics::is_block_kind`'s Java arm (private to `src/metrics/
/// mod.rs`): a `{ … }` scope is a `block` or a constructor's `constructor_body`.
fn is_block_kind(kind: &str) -> bool {
    matches!(kind, "block" | "constructor_body")
}

/// Java's non-default `switch_label`: `case`, never `default`. Mirrors
/// `metrics::is_default_label`'s own check (a `switch_label`'s first child's
/// kind is literally `"default"`).
fn is_default_label(node: Node) -> bool {
    node.child(0).is_some_and(|child| child.kind() == "default")
}

fn decision_kind(node: Node) -> Option<DecisionKind> {
    match node.kind() {
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
/// without walking back up the tree per node.
fn is_catch_body_root(node: Node) -> bool {
    is_block_kind(node.kind())
        && node
            .parent()
            .is_some_and(|parent| parent.kind() == "catch_clause")
}

/// D15's Java clone-candidate containers (`clones::statement_children`'s
/// Java arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `block`/`constructor_body`, or a
/// `switch_block_statement_group`'s direct child other than its own
/// `switch_label`.
pub(super) fn is_clone_statement(node: Node) -> bool {
    if !node.is_named() || is_comment_kind(node.kind(), LanguageFamily::Java) {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "block" | "constructor_body" => true,
        "switch_block_statement_group" => node.kind() != "switch_label",
        _ => false,
    }
}

/// The known Java damage class (`nsd-plan-final.md`'s *The 16 parse
/// failures*): a varargs parameter's annotation, e.g.
/// `void m(Class<?> @Nullable ... cs)`, produces an `ERROR` node directly
/// inside `formal_parameters`. Anything else `ERROR`/`MISSING` falls back to
/// `Unclassified` rather than going untyped.
fn classify_damage(node: Node) -> Option<DamageKind> {
    if node.is_error()
        && node
            .parent()
            .is_some_and(|parent| parent.kind() == "formal_parameters")
    {
        return Some(DamageKind::JavaVarargsAnnotation);
    }
    if node.is_error() || node.is_missing() {
        return Some(DamageKind::Unclassified);
    }
    None
}

pub(super) fn classify(node: Node, _source: &str) -> Classification {
    Classification {
        decision: decision_kind(node),
        is_terminator: TERMINATOR_KINDS.contains(&node.kind()),
        in_block: node
            .parent()
            .is_some_and(|parent| is_block_kind(parent.kind())),
        is_catch_body_root: is_catch_body_root(node),
        is_clone_statement: is_clone_statement(node),
        damage: classify_damage(node),
    }
}

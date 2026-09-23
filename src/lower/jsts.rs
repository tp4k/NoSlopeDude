//! The JS/TS lowering: every `node.kind()` match this stream produces for a
//! JS/TS file, isolated from `src/lower/java.rs`'s own table -- see that
//! file's doc comment for why the split exists.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{DamageKind, DecisionKind};
use crate::model::LanguageFamily;

use super::Classification;

/// D22/D7's terminator kinds: identical node-kind literals to Java's own
/// table, kept as a separate copy since each lowering owns its table
/// independently.
const TERMINATOR_KINDS: &[&str] = &[
    "return_statement",
    "break_statement",
    "continue_statement",
    "throw_statement",
];

/// Mirrors `metrics::is_block_kind`'s JS/TS arm (private to `src/metrics/
/// mod.rs`): a `{ … }` scope is a `statement_block` only -- deliberately not
/// `program`, the top-level module scope, which is never itself a braced
/// block.
fn is_block_kind(kind: &str) -> bool {
    kind == "statement_block"
}

fn decision_kind(node: Node) -> Option<DecisionKind> {
    match node.kind() {
        "if_statement" => Some(DecisionKind::Branch),
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
            Some(DecisionKind::Loop)
        }
        "switch_case" => Some(DecisionKind::Case),
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

/// D15's JS/TS clone-candidate containers (`clones::statement_children`'s
/// JS/TS arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `statement_block` or the top-level
/// `program`, or a `switch_case`/`switch_default`'s `body`-field child.
pub(super) fn is_clone_statement(node: Node) -> bool {
    if !node.is_named() || is_comment_kind(node.kind(), LanguageFamily::JsTs) {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "statement_block" | "program" => true,
        "switch_case" | "switch_default" => {
            let mut cursor = parent.walk();
            let found = parent
                .children_by_field_name("body", &mut cursor)
                .any(|child| child.id() == node.id());
            found
        }
        _ => false,
    }
}

/// The two known JS/TS damage classes (`nsd-plan-final.md`'s *The 16 parse
/// failures*): a TS `using` declaration used as a bare parameter name
/// produces an `ERROR` node directly inside `formal_parameters` whose own
/// first child's kind is literally the `using` keyword token; an
/// unterminated `&` inside a JSX attribute string produces an `ERROR` node
/// whose parent is a `string` and whose own first child's kind is literally
/// `"&"`. Anything else `ERROR`/`MISSING` falls back to `Unclassified`
/// rather than going untyped.
fn classify_damage(node: Node) -> Option<DamageKind> {
    if node.is_error() {
        let first_child_kind = node.child(0).map(|child| child.kind());
        let parent_kind = node.parent().map(|parent| parent.kind());
        if parent_kind == Some("formal_parameters") && first_child_kind == Some("using") {
            return Some(DamageKind::TsUsingParameterName);
        }
        if parent_kind == Some("string") && first_child_kind == Some("&") {
            return Some(DamageKind::JsxUnterminatedEntity);
        }
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

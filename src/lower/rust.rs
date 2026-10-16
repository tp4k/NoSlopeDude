//! The Rust lowering: every `node.kind()` match this stream produces for a
//! Rust file, isolated from the Java and JS/TS tables the same way those two
//! are isolated from each other -- only this file and `src/lower/mod.rs`
//! (via `Classification`) ever read a Rust grammar string. The IR mapping is
//! written out in `docs/rust-lowering.md`.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{
    CallableKind, DamageKind, DecisionKind, OwnerKind, OwnerSegment, Span, TerminatorKind,
};
use crate::model::LanguageFamily;

use super::{CallableInfo, Classification};

/// The anonymous token a bare wildcard pattern is made of.
const WILDCARD_TOKEN: &str = "_";

/// `return`, `break` and `continue` are expressions in Rust; `panic!` is a
/// macro invocation, not a terminator, and there is no `throw`.
fn terminator_kind(kind: &str) -> Option<TerminatorKind> {
    match kind {
        "return_expression" => Some(TerminatorKind::Return),
        "break_expression" => Some(TerminatorKind::Break),
        "continue_expression" => Some(TerminatorKind::Continue),
        _ => None,
    }
}

/// The Rust callable kinds. A trait's `function_signature_item` has no body
/// and a different grammar kind, so it never reaches this table.
fn callable_kind_for(kind: &str) -> Option<CallableKind> {
    match kind {
        "function_item" => Some(CallableKind::RustFunction),
        "closure_expression" => Some(CallableKind::RustClosure),
        _ => None,
    }
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

/// Whitespace-normalized text: runs of whitespace collapse to one space.
fn normalized_text(node: Node, source: &str) -> String {
    node_text(node, source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A `function_item`'s own `name` field; a closure takes the `pattern` of the
/// `let` declaration it is the value of, else it is anonymous.
fn declared_name(node: Node, parent: Option<Node>, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, source));
    }
    let parent = parent?;
    if parent.kind() != "let_declaration" {
        return None;
    }
    let pattern = parent.child_by_field_name("pattern")?;
    Some(node_text(pattern, source))
}

fn resolve_name(node: Node, parent: Option<Node>, source: &str) -> String {
    declared_name(node, parent, source)
        .unwrap_or_else(|| format!("<anonymous>@{}", node.start_position().row + 1))
}

/// The whitespace-normalized `type` text of every typed parameter, in
/// declaration order. `self_parameter` and untyped closure parameters expose
/// no `type` field and are skipped.
fn callable_signature(node: Node, source: &str) -> Vec<String> {
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters
        .children(&mut cursor)
        .filter(|parameter| parameter.kind() == "parameter")
        .filter_map(|parameter| {
            let type_node = parameter.child_by_field_name("type")?;
            Some(normalized_text(type_node, source))
        })
        .collect()
}

/// `impl Trait for Type` -> `Trait for Type`; an inherent `impl Type` ->
/// `Type`. Both are whitespace-normalized, so two trait impls on one type
/// keep distinct owner names.
fn impl_owner_name(node: Node, source: &str) -> Option<String> {
    let type_text = normalized_text(node.child_by_field_name("type")?, source);
    match node.child_by_field_name("trait") {
        Some(trait_node) => Some(format!(
            "{} for {type_text}",
            normalized_text(trait_node, source)
        )),
        None => Some(type_text),
    }
}

fn owner_segment_for_type(kind: &str, node: Node, source: &str) -> Option<OwnerSegment> {
    match kind {
        "mod_item" => Some(OwnerSegment {
            kind: OwnerKind::Namespace,
            name: node
                .child_by_field_name("name")
                .map(|name_node| node_text(name_node, source)),
        }),
        "impl_item" => Some(OwnerSegment {
            kind: OwnerKind::NamedType,
            name: impl_owner_name(node, source),
        }),
        "struct_item" | "enum_item" | "trait_item" => Some(OwnerSegment {
            kind: OwnerKind::NamedType,
            name: node
                .child_by_field_name("name")
                .map(|name_node| node_text(name_node, source)),
        }),
        _ => None,
    }
}

fn callable_info(
    node: Node,
    kind: &str,
    parent: Option<Node>,
    source: &str,
) -> Option<CallableInfo> {
    let callable_kind = callable_kind_for(kind)?;
    let body = node.child_by_field_name("body")?;
    Some(CallableInfo {
        body_span: Span::from_node(body),
        name: resolve_name(node, parent, source),
        kind: callable_kind,
        is_anonymous: declared_name(node, parent, source).is_none(),
        signature: callable_signature(node, source),
    })
}

/// The self-is-block predicate: a `{ ... }` scope is a `block`.
fn is_block_kind(kind: &str) -> Option<&'static str> {
    if kind == "block" {
        Some("block")
    } else {
        None
    }
}

/// A `match_arm`'s pattern is a bare wildcard when its `match_pattern` holds
/// the `_` token and nothing else -- a guarded `_ if cond` is not bare.
fn is_bare_wildcard_arm(arm: Node) -> bool {
    let Some(pattern) = arm.child_by_field_name("pattern") else {
        return false;
    };
    pattern.child_count() == 1
        && pattern
            .child(0)
            .is_some_and(|only| only.kind() == WILDCARD_TOKEN)
}

/// `&&` and `||` inside a `binary_expression`; any other operator is no
/// decision.
fn operator_decision(node: Node) -> Option<DecisionKind> {
    match operator_text(node) {
        Some("&&") => Some(DecisionKind::And),
        Some("||") => Some(DecisionKind::Or),
        _ => None,
    }
}

/// The `match` constructs: a non-wildcard arm is a `Case`, and a guard (the
/// `condition` of an arm's `match_pattern`) is a `Branch`.
fn match_decision(node: Node, kind: &str) -> Option<DecisionKind> {
    match kind {
        "match_arm" if !is_bare_wildcard_arm(node) => Some(DecisionKind::Case),
        "match_pattern" if node.child_by_field_name("condition").is_some() => {
            Some(DecisionKind::Branch)
        }
        _ => None,
    }
}

fn decision_kind(node: Node, kind: &str) -> Option<DecisionKind> {
    match kind {
        "if_expression" => Some(DecisionKind::Branch),
        "let_declaration" if node.child_by_field_name("alternative").is_some() => {
            Some(DecisionKind::Branch)
        }
        "while_expression" | "for_expression" => Some(DecisionKind::Loop),
        "binary_expression" => operator_decision(node),
        _ => match_decision(node, kind),
    }
}

/// A `binary_expression`'s own operator token text.
fn operator_text<'tree>(node: Node<'tree>) -> Option<&'tree str> {
    node.child_by_field_name("operator")
        .map(|child| child.kind())
}

/// Clone candidates are the named, non-comment direct children of a `block`
/// (its tail expression included).
pub(super) fn is_clone_statement(
    parent_kind: Option<&str>,
    is_named: bool,
    is_comment: bool,
) -> bool {
    is_named && !is_comment && parent_kind.is_some_and(|kind| is_block_kind(kind).is_some())
}

/// `ERROR` and `MISSING` nodes carry no stable lexical shape; Rust has no
/// dedicated damage classes.
fn classify_damage(node: Node) -> Option<DamageKind> {
    if node.is_error() || node.is_missing() {
        return Some(DamageKind::Unclassified);
    }
    None
}

/// Attribute lines are not executable source: the whole subtree of an
/// `attribute_item` or `inner_attribute_item` is silenced.
fn is_attribute_root(kind: &str) -> bool {
    matches!(kind, "attribute_item" | "inner_attribute_item")
}

pub(super) fn classify(node: Node, source: &str, parent: Option<Node>) -> Classification {
    let kind = node.kind();
    let parent_kind = parent.map(|parent| parent.kind());
    let is_named = node.is_named();
    let is_comment = is_comment_kind(kind, LanguageFamily::Rust);
    let callable = callable_info(node, kind, parent, source);
    let owner_segment = match &callable {
        Some(info) => Some(OwnerSegment {
            kind: OwnerKind::Callable,
            name: if info.is_anonymous {
                None
            } else {
                Some(info.name.clone())
            },
        }),
        None if is_named => owner_segment_for_type(kind, node, source),
        None => None,
    };
    Classification {
        decision: decision_kind(node, kind),
        terminator: terminator_kind(kind),
        in_block: parent_kind.is_some_and(|kind| is_block_kind(kind).is_some()),
        is_catch_body_root: false,
        damage: classify_damage(node),
        is_clone_statement: is_clone_statement(parent_kind, is_named, is_comment),
        is_hoisted_or_type_only: false,
        silences_executable_subtree: is_attribute_root(kind),
        block_kind: is_block_kind(kind),
        callable,
        owner_segment,
        is_comment,
        is_named,
    }
}

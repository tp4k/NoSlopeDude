//! The Java lowering: every `node.kind()` match this stream produces for a
//! Java file, isolated from `src/lower/jsts.rs`'s own table per the spec's
//! "no per-grammar strings on the IR itself" requirement -- only this file
//! and `src/lower/mod.rs` (via `Classification`) ever read a Java grammar
//! string.
//!
//! C2 (task.md): every match below is against `node.kind_id()`/
//! `child_by_field_id`, resolved through the caller's already-built
//! `KindIds` table (`kind_ids.rs`), not a per-call `node.kind()` string
//! comparison -- see that module's own doc comment for why the table is
//! built once per file rather than cached per grammar.

use tree_sitter::Node;

use crate::exec_lines::is_comment_id;
use crate::ir::{
    CallableKind, DamageKind, DecisionKind, OwnerKind, OwnerSegment, Span, TerminatorKind,
};

use super::kind_ids::KindIds;
use super::{CallableInfo, Classification};

/// D22/D7's terminator kinds: identical node-kind literals to JS/TS's own
/// table, kept as a separate copy since each lowering owns its table
/// independently.
fn terminator_kind(ids: &KindIds, kind_id: u16) -> Option<TerminatorKind> {
    if ids.is("return_statement", kind_id) {
        Some(TerminatorKind::Return)
    } else if ids.is("break_statement", kind_id) {
        Some(TerminatorKind::Break)
    } else if ids.is("continue_statement", kind_id) {
        Some(TerminatorKind::Continue)
    } else if ids.is("throw_statement", kind_id) {
        Some(TerminatorKind::Throw)
    } else {
        None
    }
}

/// D8's Java callable kinds -- this lowering's own table, not shared with
/// `src/metrics/mod.rs` (which no longer classifies callable kinds itself;
/// it consumes `IrFile::callables` directly) -- and M1-7's grammar-free
/// `CallableKind` each maps to. The sole membership gate for
/// `callable_info` below: unlike the pre-M1-7 shape (a separate
/// `CALLABLE_KINDS.contains()` check plus a would-be lookup), there is only
/// one table here, so it cannot drift out of sync with itself.
fn callable_kind_for(ids: &KindIds, kind_id: u16) -> Option<CallableKind> {
    if ids.is("method_declaration", kind_id) {
        Some(CallableKind::JavaMethod)
    } else if ids.is("constructor_declaration", kind_id) {
        Some(CallableKind::JavaConstructor)
    } else if ids.is("compact_constructor_declaration", kind_id) {
        Some(CallableKind::JavaCompactConstructor)
    } else if ids.is("static_initializer", kind_id) {
        Some(CallableKind::JavaStaticInitializer)
    } else if ids.is("lambda_expression", kind_id) {
        Some(CallableKind::JavaLambda)
    } else {
        None
    }
}

/// D8's body-node finder: every callable kind exposes it through the `body`
/// field, except `static_initializer`, whose direct `block` child carries no
/// field name.
fn callable_body<'tree>(node: Node<'tree>, ids: &KindIds) -> Option<Node<'tree>> {
    ids.field("body")
        .and_then(|field_id| node.child_by_field_id(field_id))
        .or_else(|| first_child_of_kind(node, ids, "block"))
}

fn first_child_of_kind<'tree>(node: Node<'tree>, ids: &KindIds, name: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|child| ids.is(name, child.kind_id()));
    found
}

/// D10's name resolution, minus the anonymous fallback: the node's own
/// `name` field; else the name from an enclosing `variable_declarator`,
/// `pair` or `assignment_expression`; else `None`. `resolve_name` (below)
/// and M1-7's `is_anonymous` fact both key on this exact same resolution --
/// one table, not two that could drift apart.
fn declared_name(node: Node, parent: Option<Node>, source: &str, ids: &KindIds) -> Option<String> {
    if let Some(name_node) = ids
        .field("name")
        .and_then(|field_id| node.child_by_field_id(field_id))
    {
        return Some(node_text(name_node, source));
    }
    let parent = parent?;
    let parent_kind_id = parent.kind_id();
    let field = if ids.is("variable_declarator", parent_kind_id) {
        Some("name")
    } else if ids.is("pair", parent_kind_id) {
        Some("key")
    } else if ids.is("assignment_expression", parent_kind_id) {
        Some("left")
    } else {
        None
    };
    let name_node = field
        .and_then(|field| ids.field(field))
        .and_then(|field_id| parent.child_by_field_id(field_id))?;
    Some(node_text(name_node, source))
}

/// D10: `declared_name`, else `<anonymous>@<line>`. The same table as JS/TS's
/// own copy -- see `IrCallable`'s doc comment on why both lowerings carry it.
fn resolve_name(node: Node, parent: Option<Node>, source: &str, ids: &KindIds) -> String {
    declared_name(node, parent, source, ids)
        .unwrap_or_else(|| format!("<anonymous>@{}", node.start_position().row + 1))
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

/// M1-7: the Java parameter-type signature -- every `formal_parameters` child
/// that exposes a `type` field (`formal_parameter`, `spread_parameter`), its
/// text whitespace-normalized, in declaration order. `receiver_parameter`
/// (an explicit `this` parameter) exposes no `type` field, so it is skipped
/// without a dedicated arm; `compact_constructor_declaration` and
/// `static_initializer` have no `parameters` field at all and so always
/// resolve to an empty signature via the `?` below.
///
/// Round 2 (WS-1 triage row 4): two legal overload pairs used to collide on
/// this signature alone -- `f(int)`/`f(int... xs)` (a `spread_parameter`'s
/// own trailing `"..."` was dropped, since it lives outside its `type`
/// field's span) and `g(int x)`/`g(int x[])` (a `formal_parameter`'s own
/// C-style `dimensions` field, also outside `type`'s span, was dropped
/// entirely). Both are appended onto the whitespace-normalized `type` text.
fn callable_signature(node: Node, source: &str, ids: &KindIds) -> Vec<String> {
    let Some(parameters) = ids
        .field("parameters")
        .and_then(|field_id| node.child_by_field_id(field_id))
    else {
        return Vec::new();
    };
    if !ids.is("formal_parameters", parameters.kind_id()) {
        return Vec::new();
    }
    let type_field = ids.field("type");
    let dimensions_field = ids.field("dimensions");
    let mut cursor = parameters.walk();
    parameters
        .children(&mut cursor)
        .filter_map(|parameter| {
            let type_node =
                type_field.and_then(|field_id| parameter.child_by_field_id(field_id))?;
            let mut text = node_text(type_node, source)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if ids.is("spread_parameter", parameter.kind_id()) {
                text.push_str("...");
            } else if let Some(dimensions) =
                dimensions_field.and_then(|field_id| parameter.child_by_field_id(field_id))
            {
                text.extend(node_text(dimensions, source).split_whitespace());
            }
            Some(text)
        })
        .collect()
}

/// M1-7's Java owner-chain segment for a node that is itself a named type or
/// an anonymous class body (never a callable -- `callable_info`'s own
/// `CallableInfo` already carries that segment's facts, reused directly by
/// `classify` below rather than recomputed here). Round 2 (WS-1 triage rows
/// 3/4): `annotation_type_declaration` (an `@interface`) is a named type too;
/// an `enum_constant`'s own `body` field is a `class_body` node, the same
/// grammar kind an anonymous `object_creation_expression` body uses, and JLS
/// §8.9.1 calls it exactly that -- an anonymous class body, not a member of
/// the enum's own `NamedType` -- so both parent kinds map to
/// `AnonymousClassBody`, distinguishing `PLUS { … }`'s own overriding methods
/// from the enum's shared ones.
fn owner_segment_for_type(
    ids: &KindIds,
    kind_id: u16,
    parent_kind_id: Option<u16>,
    node: Node,
    source: &str,
) -> Option<OwnerSegment> {
    let is_named_type = ids.is("class_declaration", kind_id)
        || ids.is("interface_declaration", kind_id)
        || ids.is("enum_declaration", kind_id)
        || ids.is("record_declaration", kind_id)
        || ids.is("annotation_type_declaration", kind_id);
    if is_named_type {
        let name = ids
            .field("name")
            .and_then(|field_id| node.child_by_field_id(field_id))
            .map(|name_node| node_text(name_node, source));
        return Some(OwnerSegment {
            kind: OwnerKind::NamedType,
            name,
        });
    }
    if ids.is("class_body", kind_id)
        && parent_kind_id.is_some_and(|parent_kind_id| {
            ids.is("object_creation_expression", parent_kind_id)
                || ids.is("enum_constant", parent_kind_id)
        })
    {
        return Some(OwnerSegment {
            kind: OwnerKind::AnonymousClassBody,
            name: None,
        });
    }
    None
}

fn callable_info(
    node: Node,
    kind_id: u16,
    parent: Option<Node>,
    source: &str,
    ids: &KindIds,
) -> Option<CallableInfo> {
    let callable_kind = callable_kind_for(ids, kind_id)?;
    let body = callable_body(node, ids)?;
    Some(CallableInfo {
        body_span: Span::from_node(body),
        name: resolve_name(node, parent, source, ids),
        kind: callable_kind,
        is_anonymous: declared_name(node, parent, source, ids).is_none(),
        signature: callable_signature(node, source, ids),
    })
}

/// The self-is-block predicate for Java: a `{ … }` scope is a `block` or a
/// constructor's `constructor_body`. `src/metrics/mod.rs` no longer carries
/// its own copy of this classification -- it consumes `IrFile::blocks`
/// directly. Returns the matched arm's own literal rather than a
/// bool: `Node::kind()` in tree-sitter 0.27 borrows from `node`'s own
/// lifetime rather than promising `'static`, but `IrBlock::kind` is
/// `&'static str` (`model::SyntaxBlock::kind` must not move), so the caller
/// needs the match arm's `'static` literal, not `node.kind()` itself.
fn is_block_kind(ids: &KindIds, kind_id: u16) -> Option<&'static str> {
    if ids.is("block", kind_id) {
        Some("block")
    } else if ids.is("constructor_body", kind_id) {
        Some("constructor_body")
    } else {
        None
    }
}

/// True for Java's `default` `switch_label`: its first child's kind is
/// literally `"default"`. `decision_kind` (below) counts every other
/// `switch_label` as a `Case`.
fn is_default_label(node: Node, ids: &KindIds) -> bool {
    node.child(0)
        .is_some_and(|child| ids.is("default", child.kind_id()))
}

fn decision_kind(node: Node, ids: &KindIds, kind_id: u16) -> Option<DecisionKind> {
    if ids.is("if_statement", kind_id) {
        return Some(DecisionKind::Branch);
    }
    if ids.is("for_statement", kind_id)
        || ids.is("enhanced_for_statement", kind_id)
        || ids.is("while_statement", kind_id)
        || ids.is("do_statement", kind_id)
    {
        return Some(DecisionKind::Loop);
    }
    if ids.is("switch_label", kind_id) {
        return if is_default_label(node, ids) {
            None
        } else {
            Some(DecisionKind::Case)
        };
    }
    if ids.is("catch_clause", kind_id) {
        return Some(DecisionKind::Catch);
    }
    if ids.is("ternary_expression", kind_id) {
        return Some(DecisionKind::Ternary);
    }
    if ids.is("binary_expression", kind_id) {
        return match operator_kind(node, ids) {
            Some(operator_id) if ids.is("&&", operator_id) => Some(DecisionKind::And),
            Some(operator_id) if ids.is("||", operator_id) => Some(DecisionKind::Or),
            _ => None,
        };
    }
    None
}

/// A `binary_expression`'s own operator token, via the `operator` field both
/// grammars expose it under -- its numeric kind id, not its text.
fn operator_kind(node: Node, ids: &KindIds) -> Option<u16> {
    ids.field("operator")
        .and_then(|field_id| node.child_by_field_id(field_id))
        .map(|operator| operator.kind_id())
}

/// Whether `node` itself is the block directly forming a `catch` clause's
/// body -- an O(1) check; `src/lower/mod.rs`'s `build_ir` combines this with
/// the parent's own already-computed flag to answer "or sits inside it"
/// without walking back up the tree per node. `parent_kind_id` is the
/// caller's already-threaded parent (`build_ir` passes it down the
/// traversal instead of calling `node.parent()`, which in tree-sitter
/// 0.25.10 restarts at the tree root and descends, turning one linear tree
/// build into `Θ(depth)` work per node).
fn is_catch_body_root(ids: &KindIds, kind_id: u16, parent_kind_id: Option<u16>) -> bool {
    is_block_kind(ids, kind_id).is_some()
        && parent_kind_id.is_some_and(|parent_kind_id| ids.is("catch_clause", parent_kind_id))
}

/// D15's Java clone-candidate containers (`clones::statement_children`'s
/// Java arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `block`/`constructor_body`, or a
/// `switch_block_statement_group`'s direct child other than its own
/// `switch_label`. Takes the already-threaded `parent_kind_id` rather than
/// calling `node.parent()` -- see `is_catch_body_root`'s doc comment -- and
/// the caller's own already-computed `is_named`/`is_comment` (see
/// `classify`'s own doc comment: each is computed exactly once per node and
/// threaded into every helper, rather than re-derived here). Production: its
/// result is `IrNode::is_clone_statement`.
pub(super) fn is_clone_statement(
    ids: &KindIds,
    kind_id: u16,
    parent_kind_id: Option<u16>,
    is_named: bool,
    is_comment: bool,
) -> bool {
    if !is_named || is_comment {
        return false;
    }
    let Some(parent_kind_id) = parent_kind_id else {
        return false;
    };
    if ids.is("block", parent_kind_id) || ids.is("constructor_body", parent_kind_id) {
        return true;
    }
    if ids.is("switch_block_statement_group", parent_kind_id) {
        return !ids.is("switch_label", kind_id);
    }
    false
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
/// `node.parent()`. `ids` is that same `build_ir`'s per-file `KindIds` table
/// (C2), built once from `file.tree.language()`; `kind_id`/`parent_kind_id`/
/// `is_named`/`is_comment` are each computed exactly once here (`kind_id()`,
/// not `node.kind()` -- an FFI call plus a UTF-8 validation) and threaded
/// into every helper, rather than every helper re-deriving them
/// independently.
pub(super) fn classify(
    node: Node,
    source: &str,
    parent: Option<Node>,
    ids: &KindIds,
) -> Classification {
    let kind_id = node.kind_id();
    let parent_kind_id = parent.map(|parent| parent.kind_id());
    let is_named = node.is_named();
    let is_comment = is_comment_id(kind_id, ids);
    let callable = callable_info(node, kind_id, parent, source, ids);
    // M1-7: a callable node's own owner-chain segment reuses the `Callable`
    // facts `callable_info` just computed (its kind and declared-name-ness),
    // rather than re-deriving them; every other owner-kind node (named type,
    // anonymous class body) goes through `owner_segment_for_type`.
    let owner_segment = match &callable {
        Some(info) => Some(OwnerSegment {
            kind: OwnerKind::Callable,
            name: if info.is_anonymous {
                None
            } else {
                Some(info.name.clone())
            },
        }),
        None => owner_segment_for_type(ids, kind_id, parent_kind_id, node, source),
    };
    Classification {
        decision: decision_kind(node, ids, kind_id),
        terminator: terminator_kind(ids, kind_id),
        in_block: parent_kind_id
            .is_some_and(|parent_kind_id| is_block_kind(ids, parent_kind_id).is_some()),
        is_catch_body_root: is_catch_body_root(ids, kind_id, parent_kind_id),
        damage: classify_damage(node),
        is_clone_statement: is_clone_statement(ids, kind_id, parent_kind_id, is_named, is_comment),
        is_hoisted_or_type_only: false,
        block_kind: is_block_kind(ids, kind_id),
        callable,
        owner_segment,
        is_comment,
        is_named,
    }
}

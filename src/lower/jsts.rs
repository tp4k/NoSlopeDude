//! The JS/TS lowering: every `node.kind()` match this stream produces for a
//! JS/TS file, isolated from `src/lower/java.rs`'s own table -- see that
//! file's doc comment for why the split exists.
//!
//! C2 (task.md): see `src/lower/java.rs`'s own doc comment -- every match
//! below is against `node.kind_id()`/`child_by_field_id` through the
//! caller's `KindIds` table, not a per-call `node.kind()` string comparison.

use tree_sitter::Node;

use crate::exec_lines::is_comment_id;
use crate::ir::{
    CallableKind, DamageKind, DecisionKind, OwnerKind, OwnerSegment, Span, TerminatorKind,
};

use super::kind_ids::KindIds;
use super::{CallableInfo, Classification};

/// D22/D7's terminator kinds: identical node-kind literals to Java's own
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

/// D8's JS/TS callable kinds -- this lowering's own table, not shared with
/// `src/metrics/mod.rs` (which no longer classifies callable kinds itself;
/// see `java::callable_kind_for`'s doc comment) -- and M1-7's grammar-free
/// `CallableKind` each maps to; see that same doc comment for why this is
/// the sole membership gate.
fn callable_kind_for(ids: &KindIds, kind_id: u16) -> Option<CallableKind> {
    if ids.is("function_declaration", kind_id) {
        Some(CallableKind::JsFunctionDeclaration)
    } else if ids.is("generator_function_declaration", kind_id) {
        Some(CallableKind::JsGeneratorFunctionDeclaration)
    } else if ids.is("function_expression", kind_id) {
        Some(CallableKind::JsFunctionExpression)
    } else if ids.is("arrow_function", kind_id) {
        Some(CallableKind::JsArrowFunction)
    } else if ids.is("method_definition", kind_id) {
        Some(CallableKind::JsMethodDefinition)
    } else {
        None
    }
}

/// D8's body-node finder: every JS/TS callable kind exposes its body through
/// the `body` field (no Java-style fieldless fallback needed on this side).
fn callable_body<'tree>(node: Node<'tree>, ids: &KindIds) -> Option<Node<'tree>> {
    ids.field("body")
        .and_then(|field_id| node.child_by_field_id(field_id))
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

/// D10: `declared_name`, else `<anonymous>@<line>`. The same table as Java's
/// own copy -- see `IrCallable`'s doc comment on why both lowerings carry it.
fn resolve_name(node: Node, parent: Option<Node>, source: &str, ids: &KindIds) -> String {
    declared_name(node, parent, source, ids)
        .unwrap_or_else(|| format!("<anonymous>@{}", node.start_position().row + 1))
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

/// M1-7's JS/TS owner-chain segment for a node that is itself a named type or
/// a namespace (never a callable -- see `java::owner_segment_for_type`'s own
/// doc comment for why). `class_declaration` and `abstract_class_declaration`
/// are always named in this grammar; the `class` expression form is
/// optionally named, so it falls back to `AnonymousClassBody` when its own
/// `name` field is absent. `internal_module` (the `namespace` keyword) and
/// `module` (the `module` keyword -- a distinct grammar node from
/// `internal_module`, not an alternate spelling of it) both always carry a
/// required `name` field, and both are TS's namespace concept (round 2, WS-1
/// triage row 2: `module M { … }` used to fall through this match entirely
/// and get no owner segment at all).
fn owner_segment_for_type(
    ids: &KindIds,
    kind_id: u16,
    node: Node,
    source: &str,
) -> Option<OwnerSegment> {
    if ids.is("class_declaration", kind_id)
        || ids.is("class", kind_id)
        || ids.is("abstract_class_declaration", kind_id)
    {
        let name = ids
            .field("name")
            .and_then(|field_id| node.child_by_field_id(field_id))
            .map(|name_node| node_text(name_node, source));
        return Some(match name {
            Some(name) => OwnerSegment {
                kind: OwnerKind::NamedType,
                name: Some(name),
            },
            None => OwnerSegment {
                kind: OwnerKind::AnonymousClassBody,
                name: None,
            },
        });
    }
    if ids.is("internal_module", kind_id) || ids.is("module", kind_id) {
        let name = ids
            .field("name")
            .and_then(|field_id| node.child_by_field_id(field_id))
            .map(|name_node| node_text(name_node, source));
        return Some(OwnerSegment {
            kind: OwnerKind::Namespace,
            name,
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
        // JS/TS signatures are always empty (answer 4): untyped JS has no
        // parameter types, and a TS overload signature without a body is
        // not a callable.
        signature: Vec::new(),
    })
}

/// D22 exception, JS/TS only (`rules::is_hoisted_or_type_only`, re-derived
/// here since that function is private): a hoisted function declaration or
/// a type-only declaration, exempt from the unreachable-after-return rule.
fn is_hoisted_or_type_only(ids: &KindIds, kind_id: u16) -> bool {
    ids.is("function_declaration", kind_id)
        || ids.is("generator_function_declaration", kind_id)
        || ids.is("type_alias_declaration", kind_id)
        || ids.is("interface_declaration", kind_id)
}

/// The self-is-block predicate for JS/TS (see `java::is_block_kind`'s own
/// doc comment on why `src/metrics/mod.rs` no longer carries its own copy):
/// a `{ … }` scope is a `statement_block` only -- deliberately not
/// `program`, the top-level module scope, which is never itself a braced
/// block. Returns the matched arm's own literal rather than a bool -- see
/// `java::is_block_kind`'s doc comment for why.
fn is_block_kind(ids: &KindIds, kind_id: u16) -> Option<&'static str> {
    if ids.is("statement_block", kind_id) {
        Some("statement_block")
    } else {
        None
    }
}

fn decision_kind(node: Node, ids: &KindIds, kind_id: u16) -> Option<DecisionKind> {
    if ids.is("if_statement", kind_id) {
        return Some(DecisionKind::Branch);
    }
    if ids.is("for_statement", kind_id)
        || ids.is("for_in_statement", kind_id)
        || ids.is("while_statement", kind_id)
        || ids.is("do_statement", kind_id)
    {
        return Some(DecisionKind::Loop);
    }
    if ids.is("switch_case", kind_id) {
        return Some(DecisionKind::Case);
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
            // `??` shares `Or`'s weight (D7: "`&&`/`||`/`??` add 1", one
            // shared arm pre-IR) -- not a new variant, since
            // `metrics::decision_weight`'s exhaustive match has no wildcard
            // arm and lives outside this stream's fence.
            Some(operator_id) if ids.is("||", operator_id) || ids.is("??", operator_id) => {
                Some(DecisionKind::Or)
            }
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

/// D15's JS/TS clone-candidate containers (`clones::statement_children`'s
/// JS/TS arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `statement_block` or the top-level
/// `program`, or a `switch_case`/`switch_default`'s `body`-field child.
/// Takes the already-threaded `parent_kind_id` rather than calling
/// `node.parent()` -- see `is_catch_body_root`'s doc comment -- and the
/// caller's own already-computed `is_named`/`is_comment` (see `classify`'s
/// own doc comment). The `switch_case`/`switch_default` arm reads
/// `field_id` -- `build_ir`'s cursor is already positioned on this exact
/// node when it is opened, so its field id relative to its parent is an O(1)
/// `TreeCursor::field_id()` read rather than a `children_by_field_name`
/// re-scan of the parent (this arm used to cost `Θ(K)` per node, `Θ(K²)` per
/// case body). Production: its result is `IrNode::is_clone_statement`.
pub(super) fn is_clone_statement(
    ids: &KindIds,
    parent_kind_id: Option<u16>,
    field_id: Option<u16>,
    is_named: bool,
    is_comment: bool,
) -> bool {
    if !is_named || is_comment {
        return false;
    }
    let Some(parent_kind_id) = parent_kind_id else {
        return false;
    };
    if ids.is("statement_block", parent_kind_id) || ids.is("program", parent_kind_id) {
        return true;
    }
    if ids.is("switch_case", parent_kind_id) || ids.is("switch_default", parent_kind_id) {
        return field_id.is_some() && field_id == ids.field("body");
    }
    false
}

/// The two known JS/TS damage classes (`nsd-plan-final.md`'s *The 16 parse
/// failures*): a TS `using` declaration used as a bare parameter name
/// produces an `ERROR` node directly inside `formal_parameters` whose own
/// first child's kind is literally the `using` keyword token; an
/// unterminated `&` inside a JSX attribute string produces an `ERROR` node
/// whose parent is a `string` and whose own first child's kind is literally
/// `"&"`. Anything else `ERROR`/`MISSING` falls back to `Unclassified`
/// rather than going untyped. Takes the already-threaded `parent_kind_id` --
/// see `is_catch_body_root`'s doc comment.
fn classify_damage(node: Node, ids: &KindIds, parent_kind_id: Option<u16>) -> Option<DamageKind> {
    if node.is_error() {
        let first_child_kind_id = node.child(0).map(|child| child.kind_id());
        if parent_kind_id.is_some_and(|parent_kind_id| ids.is("formal_parameters", parent_kind_id))
            && first_child_kind_id.is_some_and(|child_kind_id| ids.is("using", child_kind_id))
        {
            return Some(DamageKind::TsUsingParameterName);
        }
        if parent_kind_id.is_some_and(|parent_kind_id| ids.is("string", parent_kind_id))
            && first_child_kind_id.is_some_and(|child_kind_id| ids.is("&", child_kind_id))
        {
            return Some(DamageKind::JsxUnterminatedEntity);
        }
    }
    if node.is_error() || node.is_missing() {
        return Some(DamageKind::Unclassified);
    }
    None
}

/// `parent` is the tree-sitter `Node` `src/lower/mod.rs`'s `build_ir` already
/// holds for this node's parent (threaded down the traversal in a stack
/// mirroring its own node stack), so nothing below this point calls
/// `node.parent()`. `field_id` is that same `build_ir`'s cursor's own field
/// id for this exact node, an O(1) `TreeCursor::field_id()` read rather than
/// a re-scan. `ids` is `build_ir`'s per-file `KindIds` table (C2), built once
/// from `file.tree.language()`. `kind_id`/`parent_kind_id`/`is_named`/
/// `is_comment` are each computed exactly once here (`kind_id()`, not
/// `node.kind()`) and threaded into every helper, rather than every helper
/// re-deriving them independently.
pub(super) fn classify(
    node: Node,
    source: &str,
    parent: Option<Node>,
    field_id: Option<u16>,
    ids: &KindIds,
) -> Classification {
    let kind_id = node.kind_id();
    let parent_kind_id = parent.map(|parent| parent.kind_id());
    let is_named = node.is_named();
    let is_comment = is_comment_id(kind_id, ids);
    let callable = callable_info(node, kind_id, parent, source, ids);
    // M1-7: see `java::classify`'s own comment on this same pattern.
    let owner_segment = match &callable {
        Some(info) => Some(OwnerSegment {
            kind: OwnerKind::Callable,
            name: if info.is_anonymous {
                None
            } else {
                Some(info.name.clone())
            },
        }),
        // WS-1 triage row 1 (round 3): `class`/`module` are also the kind
        // strings of the anonymous keyword leaf tokens (`is_named() ==
        // false`), which share their kind string with the named
        // `class_declaration`/`class` expression/`module` arms above --
        // gating on `is_named` keeps the keyword leaf from getting a
        // phantom owner segment of its own.
        None if is_named => owner_segment_for_type(ids, kind_id, node, source),
        None => None,
    };
    Classification {
        decision: decision_kind(node, ids, kind_id),
        terminator: terminator_kind(ids, kind_id),
        in_block: parent_kind_id
            .is_some_and(|parent_kind_id| is_block_kind(ids, parent_kind_id).is_some()),
        is_catch_body_root: is_catch_body_root(ids, kind_id, parent_kind_id),
        damage: classify_damage(node, ids, parent_kind_id),
        is_clone_statement: is_clone_statement(ids, parent_kind_id, field_id, is_named, is_comment),
        is_hoisted_or_type_only: is_hoisted_or_type_only(ids, kind_id),
        block_kind: is_block_kind(ids, kind_id),
        callable,
        owner_segment,
        is_comment,
        is_named,
    }
}

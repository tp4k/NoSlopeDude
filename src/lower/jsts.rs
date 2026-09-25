//! The JS/TS lowering: every `node.kind()` match this stream produces for a
//! JS/TS file, isolated from `src/lower/java.rs`'s own table -- see that
//! file's doc comment for why the split exists.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{
    CallableKind, DamageKind, DecisionKind, OwnerKind, OwnerSegment, Span, TerminatorKind,
};
use crate::model::LanguageFamily;

use super::{CallableInfo, Classification};

/// D22/D7's terminator kinds: identical node-kind literals to Java's own
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

/// D8's JS/TS callable kinds (`metrics::JSTS_CALLABLE_KINDS`, private to a
/// module this stream may not touch -- re-derived here rather than shared),
/// and M1-7's grammar-free `CallableKind` each maps to -- see
/// `java::callable_kind_for`'s doc comment for why this is the sole
/// membership gate.
fn callable_kind_for(kind: &str) -> Option<CallableKind> {
    match kind {
        "function_declaration" => Some(CallableKind::JsFunctionDeclaration),
        "generator_function_declaration" => Some(CallableKind::JsGeneratorFunctionDeclaration),
        "function_expression" => Some(CallableKind::JsFunctionExpression),
        "arrow_function" => Some(CallableKind::JsArrowFunction),
        "method_definition" => Some(CallableKind::JsMethodDefinition),
        _ => None,
    }
}

/// D8's body-node finder: every JS/TS callable kind exposes its body through
/// the `body` field (no Java-style fieldless fallback needed on this side).
fn callable_body(node: Node) -> Option<Node> {
    node.child_by_field_name("body")
}

/// D10's name resolution, minus the anonymous fallback: the node's own
/// `name` field; else the name from an enclosing `variable_declarator`,
/// `pair` or `assignment_expression`; else `None`. `resolve_name` (below)
/// and M1-7's `is_anonymous` fact both key on this exact same resolution --
/// one table, not two that could drift apart.
fn declared_name(node: Node, parent: Option<Node>, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, source));
    }
    let parent = parent?;
    let field = match parent.kind() {
        "variable_declarator" => Some("name"),
        "pair" => Some("key"),
        "assignment_expression" => Some("left"),
        _ => None,
    };
    let name_node = field.and_then(|field| parent.child_by_field_name(field))?;
    Some(node_text(name_node, source))
}

/// D10: `declared_name`, else `<anonymous>@<line>`. The same table as Java's
/// own copy -- see `IrCallable`'s doc comment on why both lowerings carry it.
fn resolve_name(node: Node, parent: Option<Node>, source: &str) -> String {
    declared_name(node, parent, source)
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
fn owner_segment_for_type(kind: &str, node: Node, source: &str) -> Option<OwnerSegment> {
    match kind {
        "class_declaration" | "class" | "abstract_class_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|name_node| node_text(name_node, source));
            Some(match name {
                Some(name) => OwnerSegment {
                    kind: OwnerKind::NamedType,
                    name: Some(name),
                },
                None => OwnerSegment {
                    kind: OwnerKind::AnonymousClassBody,
                    name: None,
                },
            })
        }
        "internal_module" | "module" => {
            let name = node
                .child_by_field_name("name")
                .map(|name_node| node_text(name_node, source));
            Some(OwnerSegment {
                kind: OwnerKind::Namespace,
                name,
            })
        }
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
    let body = callable_body(node)?;
    Some(CallableInfo {
        body_span: Span::from_node(body),
        name: resolve_name(node, parent, source),
        kind: callable_kind,
        is_anonymous: declared_name(node, parent, source).is_none(),
        // JS/TS signatures are always empty (answer 4): untyped JS has no
        // parameter types, and a TS overload signature without a body is
        // not a callable.
        signature: Vec::new(),
    })
}

/// D22 exception, JS/TS only (`rules::is_hoisted_or_type_only`, re-derived
/// here since that function is private): a hoisted function declaration or
/// a type-only declaration, exempt from the unreachable-after-return rule.
fn is_hoisted_or_type_only(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "type_alias_declaration"
            | "interface_declaration"
    )
}

/// Mirrors `metrics::is_block_kind`'s JS/TS arm (private to `src/metrics/
/// mod.rs`): a `{ … }` scope is a `statement_block` only -- deliberately not
/// `program`, the top-level module scope, which is never itself a braced
/// block. Returns the matched arm's own literal rather than a bool -- see
/// `java::is_block_kind`'s doc comment for why.
fn is_block_kind(kind: &str) -> Option<&'static str> {
    if kind == "statement_block" {
        Some("statement_block")
    } else {
        None
    }
}

fn decision_kind(node: Node, kind: &str) -> Option<DecisionKind> {
    match kind {
        "if_statement" => Some(DecisionKind::Branch),
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
            Some(DecisionKind::Loop)
        }
        "switch_case" => Some(DecisionKind::Case),
        "catch_clause" => Some(DecisionKind::Catch),
        "ternary_expression" => Some(DecisionKind::Ternary),
        "binary_expression" => match operator_text(node) {
            Some("&&") => Some(DecisionKind::And),
            // `??` shares `Or`'s weight (D7: "`&&`/`||`/`??` add 1", one
            // shared arm pre-IR) -- not a new variant, since
            // `metrics::decision_weight`'s exhaustive match has no wildcard
            // arm and lives outside this stream's fence.
            Some("||") | Some("??") => Some(DecisionKind::Or),
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

/// D15's JS/TS clone-candidate containers (`clones::statement_children`'s
/// JS/TS arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `statement_block` or the top-level
/// `program`, or a `switch_case`/`switch_default`'s `body`-field child.
/// Takes the already-threaded `parent_kind` rather than calling
/// `node.parent()` -- see `is_catch_body_root`'s doc comment -- and the
/// caller's own already-computed `is_named`/`is_comment` (see `classify`'s
/// own doc comment). The `switch_case`/`switch_default` arm reads
/// `field_name` -- `build_ir`'s cursor is already positioned on this exact
/// node when it is opened, so its field name relative to its parent is an
/// O(1) `TreeCursor::field_name()` read rather than a `children_by_field_name`
/// re-scan of the parent (this arm used to cost `Θ(K)` per node, `Θ(K²)` per
/// case body). Production: its result is `IrNode::is_clone_statement`.
pub(super) fn is_clone_statement(
    parent_kind: Option<&str>,
    field_name: Option<&str>,
    is_named: bool,
    is_comment: bool,
) -> bool {
    if !is_named || is_comment {
        return false;
    }
    match parent_kind {
        Some("statement_block") | Some("program") => true,
        Some("switch_case") | Some("switch_default") => field_name == Some("body"),
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
/// rather than going untyped. Takes the already-threaded `parent_kind` --
/// see `is_catch_body_root`'s doc comment.
fn classify_damage(node: Node, parent_kind: Option<&str>) -> Option<DamageKind> {
    if node.is_error() {
        let first_child_kind = node.child(0).map(|child| child.kind());
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

/// `parent` is the tree-sitter `Node` `src/lower/mod.rs`'s `build_ir` already
/// holds for this node's parent (threaded down the traversal in a stack
/// mirroring its own node stack), so nothing below this point calls
/// `node.parent()`. `field_name` is that same `build_ir`'s cursor's own field
/// name for this exact node, an O(1) `TreeCursor::field_name()` read rather
/// than a re-scan. `kind`/`parent_kind`/`is_named`/`is_comment` are each
/// computed exactly once here and threaded into every helper, rather than
/// every helper re-deriving `node.kind()` (a strlen + full-UTF8-validate
/// call), `node.is_named()` or `is_comment_kind` independently.
pub(super) fn classify(
    node: Node,
    source: &str,
    parent: Option<Node>,
    field_name: Option<&str>,
) -> Classification {
    let kind = node.kind();
    let parent_kind = parent.map(|parent| parent.kind());
    let is_named = node.is_named();
    let is_comment = is_comment_kind(kind, LanguageFamily::JsTs);
    let callable = callable_info(node, kind, parent, source);
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
        None => owner_segment_for_type(kind, node, source),
    };
    Classification {
        decision: decision_kind(node, kind),
        terminator: terminator_kind(kind),
        in_block: parent_kind.is_some_and(|kind| is_block_kind(kind).is_some()),
        is_catch_body_root: is_catch_body_root(kind, parent_kind),
        damage: classify_damage(node, parent_kind),
        is_clone_statement: is_clone_statement(parent_kind, field_name, is_named, is_comment),
        is_hoisted_or_type_only: is_hoisted_or_type_only(kind),
        block_kind: is_block_kind(kind),
        callable,
        owner_segment,
        is_comment,
        is_named,
    }
}

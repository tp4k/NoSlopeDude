//! The JS/TS lowering: every `node.kind()` match this stream produces for a
//! JS/TS file, isolated from `src/lower/java.rs`'s own table -- see that
//! file's doc comment for why the split exists.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::ir::{DamageKind, DecisionKind, Span, TerminatorKind};
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
/// module this stream may not touch -- re-derived here rather than shared).
const CALLABLE_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "arrow_function",
    "method_definition",
];

/// D8's body-node finder: every JS/TS callable kind exposes its body through
/// the `body` field (no Java-style fieldless fallback needed on this side).
fn callable_body(node: Node) -> Option<Node> {
    node.child_by_field_name("body")
}

/// D10: the node's own `name` field; else the name from an enclosing
/// `variable_declarator`, `pair` or `assignment_expression`; else
/// `<anonymous>@<line>`. The same table as Java's own copy -- see
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
/// block.
fn is_block_kind(kind: &str) -> bool {
    kind == "statement_block"
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
    is_block_kind(kind) && parent_kind == Some("catch_clause")
}

/// D15's JS/TS clone-candidate containers (`clones::statement_children`'s
/// JS/TS arms, re-derived here since that function is private): a direct
/// named, non-comment child of a `statement_block` or the top-level
/// `program`, or a `switch_case`/`switch_default`'s `body`-field child.
/// Takes the already-threaded `parent`/`parent_kind` rather than calling
/// `node.parent()` -- see `is_catch_body_root`'s doc comment. The
/// `switch_case`/`switch_default` arm is the one caller in either lowering
/// that needs the parent `Node` itself, not just its kind, since
/// `children_by_field_name` is a method on `Node`. Production: its result is
/// `IrNode::is_clone_statement`.
pub(super) fn is_clone_statement(
    node: Node,
    kind: &str,
    parent: Option<Node>,
    parent_kind: Option<&str>,
) -> bool {
    if !node.is_named() || is_comment_kind(kind, LanguageFamily::JsTs) {
        return false;
    }
    match parent_kind {
        Some("statement_block") | Some("program") => true,
        Some("switch_case") | Some("switch_default") => {
            let Some(parent) = parent else { return false };
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
/// `node.parent()`. `kind`/`parent_kind` are each computed exactly once here
/// and threaded into every helper, rather than every helper re-deriving
/// `node.kind()` (a strlen + full-UTF8-validate call) independently.
pub(super) fn classify(node: Node, source: &str, parent: Option<Node>) -> Classification {
    let kind = node.kind();
    let parent_kind = parent.map(|parent| parent.kind());
    Classification {
        decision: decision_kind(node, kind),
        terminator: terminator_kind(kind),
        in_block: parent_kind.is_some_and(is_block_kind),
        is_catch_body_root: is_catch_body_root(kind, parent_kind),
        damage: classify_damage(node, parent_kind),
        is_clone_statement: is_clone_statement(node, kind, parent, parent_kind),
        is_hoisted_or_type_only: is_hoisted_or_type_only(kind),
        is_block: is_block_kind(kind),
        callable: callable_info(node, kind, parent, source),
    }
}

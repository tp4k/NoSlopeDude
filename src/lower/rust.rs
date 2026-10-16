//! The Rust lowering. Scaffold: every classification is empty until the
//! mapping in `docs/rust-lowering.md` is implemented.

use tree_sitter::Node;

use crate::exec_lines::is_comment_kind;
use crate::model::LanguageFamily;

use super::Classification;

/// Clone candidates are the named, non-comment direct children of a `block`.
pub(super) fn is_clone_statement(
    _parent_kind: Option<&str>,
    _is_named: bool,
    _is_comment: bool,
) -> bool {
    false
}

pub(super) fn classify(node: Node, _source: &str, _parent: Option<Node>) -> Classification {
    let is_comment = is_comment_kind(node.kind(), LanguageFamily::Rust);
    Classification {
        decision: None,
        terminator: None,
        in_block: false,
        is_catch_body_root: false,
        damage: None,
        is_clone_statement: is_clone_statement(None, node.is_named(), is_comment),
        is_hoisted_or_type_only: false,
        silences_executable_subtree: false,
        block_kind: None,
        callable: None,
        owner_segment: None,
        is_comment,
        is_named: node.is_named(),
    }
}

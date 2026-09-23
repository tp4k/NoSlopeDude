//! D11: the shared "executable line" predicate. Metrics (SLOC), clones
//! (redundant-line counting) and rules (flagged-line sets) all need the
//! same answer to "does this leaf's line count as executable source" —
//! previously three hand-maintained, textually identical copies, now one
//! definition. This is also the function M0b's IR lowers to as its single
//! `executable` rule (`nsd-plan-final.md` M0a step 2).

use tree_sitter::Node;

use crate::model::LanguageFamily;

/// D11: a leaf token's line counts toward SLOC when the leaf is named and
/// is not a comment.
pub(crate) fn is_comment_kind(kind: &str, language: LanguageFamily) -> bool {
    match language {
        LanguageFamily::Java => matches!(kind, "line_comment" | "block_comment"),
        LanguageFamily::JsTs => kind == "comment",
    }
}

/// Node kinds whose *bare* form (no label, no returned expression) has only
/// anonymous keyword/`;` children, so the generic `child_count() == 0` leaf
/// check misses them entirely — undercounting a line holding only `break;`,
/// `continue;` or `return;`. A `return expr;` still isn't matched here since
/// its own line is already covered by `expr`'s leaf tokens.
const BARE_CONTROL_FLOW_KINDS: &[&str] =
    &["break_statement", "continue_statement", "return_statement"];

fn is_bare_control_flow(node: Node) -> bool {
    BARE_CONTROL_FLOW_KINDS.contains(&node.kind()) && node.named_child_count() == 0
}

pub(crate) fn is_executable_leaf(node: Node, language: LanguageFamily) -> bool {
    if !node.is_named() || is_comment_kind(node.kind(), language) {
        return false;
    }
    node.child_count() == 0 || is_bare_control_flow(node)
}

/// Iterative pre-order traversal via a single reused `TreeCursor` (no
/// native call-stack growth): visits `root` and every descendant. Private
/// to this module and, now that `collect_executable_lines` (the production
/// caller) has moved to `rules::collect_ir_executable_lines`, the IR-native
/// retarget of the same D11 rule, used only by the test helpers below —
/// `#[cfg(test)]` reflects that honestly rather than leaving a
/// production-only dead-code warning for `-D warnings` to catch.
#[cfg(test)]
fn for_each_descendant<'tree>(root: Node<'tree>, mut visit: impl FnMut(Node<'tree>)) {
    let mut cursor = root.walk();
    loop {
        visit(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tree_sitter::{Node, Parser, Tree};

    use super::*;

    fn parse_java(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("java grammar");
        parser.parse(source, None).expect("java parse")
    }

    fn parse_jsts(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("javascript grammar");
        parser.parse(source, None).expect("javascript parse")
    }

    fn find_by_kind<'tree>(root: Node<'tree>, kind: &str) -> Node<'tree> {
        let mut found = None;
        for_each_descendant(root, |node| {
            if found.is_none() && node.kind() == kind {
                found = Some(node);
            }
        });
        found.unwrap_or_else(|| panic!("no {kind} node found in parsed tree"))
    }

    #[test]
    fn test_executable_leaf_rejects_comments_in_both_families() {
        let java_line = parse_java("class C {\n    void m() {\n        // note\n    }\n}\n");
        let comment = find_by_kind(java_line.root_node(), "line_comment");
        assert!(!is_executable_leaf(comment, LanguageFamily::Java));

        let java_block = parse_java("class C {\n    void m() {\n        /* note */\n    }\n}\n");
        let comment = find_by_kind(java_block.root_node(), "block_comment");
        assert!(!is_executable_leaf(comment, LanguageFamily::Java));

        let jsts = parse_jsts("function f() {\n  // note\n}\n");
        let comment = find_by_kind(jsts.root_node(), "comment");
        assert!(!is_executable_leaf(comment, LanguageFamily::JsTs));
    }

    #[test]
    fn test_bare_control_flow_is_a_leaf_but_a_returning_one_is_not() {
        let bare = parse_java("class C {\n    void m() {\n        if (true) {\n            return;\n        }\n    }\n}\n");
        let bare_return = find_by_kind(bare.root_node(), "return_statement");
        assert!(is_executable_leaf(bare_return, LanguageFamily::Java));

        let returning = parse_java("class C {\n    int m() {\n        return 1;\n    }\n}\n");
        let returning_statement = find_by_kind(returning.root_node(), "return_statement");
        assert!(!is_executable_leaf(
            returning_statement,
            LanguageFamily::Java
        ));
    }
}

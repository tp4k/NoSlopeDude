//! D11: the shared "executable line" predicate. Metrics (SLOC), clones
//! (redundant-line counting) and rules (flagged-line sets) all need the
//! same answer to "does this leaf's line count as executable source" —
//! previously three hand-maintained, textually identical copies, now one
//! definition. This is also the function M0b's IR lowers to as its single
//! `executable` rule (`nsd-plan-final.md` M0a step 2).

#[cfg(test)]
mod tests {
    use tree_sitter::{Node, Parser, Tree};

    use super::*;
    use crate::model::LanguageFamily;

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
        assert!(!is_executable_leaf(returning_statement, LanguageFamily::Java));
    }
}

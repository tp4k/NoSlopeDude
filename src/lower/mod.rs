//! M0b: the two lowerings (`nsd-plan-final.md` M0b item 5) -- plain
//! functions over tree-sitter trees, no `Frontend` trait, no plugin system.
//! `src/lower/java.rs` and `src/lower/jsts.rs` hold every grammar-specific
//! `node.kind()` match (decision classification, terminator kinds, block/
//! catch-body structure, damage classification, clone-candidate statement
//! containers); this file holds only the traversal shared by both -- one
//! iterative `TreeCursor`-based tree build (D18: no per-AST-depth
//! recursion), the same shape `metrics::walk_excluding` already uses for its
//! own flat traversal, extended here to also assemble the `IrNode` tree
//! itself rather than only visiting.

mod java;
mod jsts;

use std::path::PathBuf;

use tree_sitter::Node;

#[cfg(test)]
use crate::exec_lines::is_comment_kind;
use crate::exec_lines::is_executable_leaf;
use crate::ir::{DamageKind, DamageSpan, DecisionKind, IrNode, Span};
use crate::model::LanguageFamily;
use crate::parse::ParsedFile;

/// M0c's fingerprint keys on this alongside `ir::IR_VERSION`; bump it
/// whenever the Java lowering's classification changes what an `IrNode`
/// carries for a Java file.
pub const JAVA_LOWERING_VERSION: u32 = 1;

/// The JS/TS counterpart of `JAVA_LOWERING_VERSION`.
pub const JSTS_LOWERING_VERSION: u32 = 1;

/// One D14 separator between adjacent leaf tokens in a statement's
/// normalized stream -- the same control character
/// `clones::normalized_statement_tokens` uses as its own private
/// `TOKEN_SEPARATOR`. This lowering's `statement_token_stream` is an
/// independent re-derivation of that function (the floor prototype
/// `nsd-plan-final.md` M0b item 4 calls for), so the two must agree on this
/// exact value for the byte-for-byte comparison to mean anything; it cannot
/// be imported since the original is private to `src/clones/mod.rs`.
/// Test-only: nothing in `src/` reads a clone-candidate token stream yet
/// (`IrNode::token`, its only production reader, was removed as unread and
/// unbounded -- see `ir::IrNode`'s doc comment); this and the two helpers
/// below stay for WS-4 to build on, proven by the floor test in `mod tests`.
#[cfg(test)]
const STATEMENT_TOKEN_SEPARATOR: char = '\u{1}';

/// One lowered file: the IR tree, plus every typed damage span found while
/// building it. Nothing in this stream reads `damage` or `root` yet -- the
/// pipeline only builds this per file (M0b item 5's "wire the lowering in",
/// not "retarget an analyzer").
pub struct IrFile {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub root: IrNode,
    pub damage: Vec<DamageSpan>,
}

/// Lowers every parsed file, one rayon task per file (D21), matching the
/// upstream stages' own per-file parallelism.
pub fn lower_all(parsed_files: &[ParsedFile]) -> Vec<IrFile> {
    use rayon::prelude::*;
    parsed_files.par_iter().map(lower_file).collect()
}

/// Lowers one parsed file's whole tree into its `IrNode` root.
pub fn lower_file(file: &ParsedFile) -> IrFile {
    let mut damage = Vec::new();
    let root = build_ir(
        file.tree.root_node(),
        file.language,
        &file.source,
        &mut damage,
    );
    IrFile {
        relative_path: file.relative_path.clone(),
        language: file.language,
        root,
        damage,
    }
}

/// One node's normalized facts, before it is wrapped as an `IrNode`. Kept
/// separate from `IrNode` itself so `src/lower/java.rs` / `jsts.rs` classify
/// a node without needing to know `IrNode`'s `children` field exists.
struct Classification {
    decision: Option<DecisionKind>,
    is_terminator: bool,
    in_block: bool,
    /// Whether this exact node is the block directly forming a `catch`
    /// clause's body -- an O(1) check per node. `IrNode::in_catch_body`
    /// (also true for every node *inside* that block) is computed by
    /// `build_ir` from this plus the parent's own already-computed flag,
    /// inherited during the traversal rather than re-derived per node by
    /// walking back up to the root (which would turn one linear tree build
    /// into `Θ(depth)` work per node -- `Θ(n²)` on a file whose nesting
    /// depth scales with its size, the pathological case
    /// `test_deeply_nested_file_does_not_abort_the_scan` exists to catch).
    is_catch_body_root: bool,
    damage: Option<DamageKind>,
}

/// `parent` is `build_ir`'s already-threaded parent `Node` (see that
/// function's own doc comment for why: `node.parent()` restarts at the tree
/// root in tree-sitter 0.25.10, so threading it down the traversal keeps the
/// whole lowering `Θ(n)` instead of `Θ(n·depth)`).
fn classify(
    node: Node,
    language: LanguageFamily,
    source: &str,
    parent: Option<Node>,
) -> Classification {
    match language {
        LanguageFamily::Java => java::classify(node, source, parent),
        LanguageFamily::JsTs => jsts::classify(node, source, parent),
    }
}

/// Whether `node` is a clone-candidate container's direct statement child --
/// the same granularity `clones::statement_children`'s containers enumerate,
/// re-derived here (that function is private to `src/clones/mod.rs`, and
/// this stream may not widen it). `java::is_clone_statement`/
/// `jsts::is_clone_statement` are kept for WS-4's clone-floor work even
/// though nothing in `src/` reads their result today (the `IrNode.token`
/// field that once carried it was removed as unread/unbounded -- see
/// `ir::IrNode`'s doc comment); this dispatcher exists only for tests below,
/// which need to enumerate the tree-sitter nodes they compare the lowering's
/// tokens against. Test-only, so deriving `node.parent()` directly (rather
/// than threading it, as `build_ir` does for the production path) is fine:
/// nothing here runs against the deep-nesting perf fixture.
#[cfg(test)]
fn is_clone_statement(node: Node, language: LanguageFamily) -> bool {
    let kind = node.kind();
    let parent = node.parent();
    let parent_kind = parent.map(|parent| parent.kind());
    match language {
        LanguageFamily::Java => java::is_clone_statement(node, kind, parent_kind),
        LanguageFamily::JsTs => jsts::is_clone_statement(node, kind, parent, parent_kind),
    }
}

/// D14: one statement node's own normalized token stream -- every leaf
/// descendant in order, comments dropped, an anonymous leaf reduced to its
/// `node.kind()`, a named leaf's text preserved verbatim. Deliberately a
/// fresh implementation of the same algorithm
/// `clones::normalized_statement_tokens` uses, not a call to it: the floor
/// prototype (`nsd-plan-final.md` M0b item 4) exists to prove this
/// independent derivation agrees with the pre-IR one, not to wrap it.
/// Test-only -- see `STATEMENT_TOKEN_SEPARATOR`'s doc comment.
#[cfg(test)]
fn statement_token_stream(statement: Node, language: LanguageFamily, source: &str) -> String {
    let mut tokens = String::new();
    for_each_descendant(statement, |node| {
        if node.child_count() != 0 || is_comment_kind(node.kind(), language) {
            return;
        }
        tokens.push(STATEMENT_TOKEN_SEPARATOR);
        if node.is_named() {
            tokens.push_str(node.utf8_text(source.as_bytes()).unwrap_or(""));
        } else {
            tokens.push_str(node.kind());
        }
    });
    tokens
}

/// Iterative pre-order traversal via a single reused `TreeCursor`: visits
/// `root` and every descendant. The same shape as
/// `exec_lines::for_each_descendant` / `clones::for_each_descendant`, a
/// fresh copy here since both are private to their own modules. Test-only --
/// see `STATEMENT_TOKEN_SEPARATOR`'s doc comment; `build_ir` below has its
/// own production traversal, since it also needs to assemble a tree rather
/// than only visit.
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

/// A structurally-unreachable fallback `IrNode` for the two dead branches in
/// `build_ir` below: `stack` always holds exactly one frame per node
/// currently open between the traversal's `goto_first_child` into it and
/// its own finalization here, so it cannot be found empty, and a `TreeCursor`
/// created from `root.walk()` cannot be asked to ascend past `root` before
/// `root`'s own frame has already been finalized and returned. Returning a
/// span-only node rather than panicking keeps this scan-pipeline stage
/// consistent with D18: a defect here degrades, it does not crash the run.
fn fallback_node(root: Node) -> IrNode {
    IrNode {
        span: Span::from_node(root),
        executable: false,
        decision: None,
        is_terminator: false,
        in_block: false,
        in_catch_body: false,
        children: Vec::new(),
    }
}

/// Builds `root`'s whole `IrNode` tree in one iterative pass: a single
/// `TreeCursor` walks the tree exactly as `metrics::walk_excluding` does
/// (D18: no per-AST-depth recursion), but this traversal also assembles a
/// tree rather than only visiting, via an explicit stack of in-progress
/// `IrNode`s mirroring the cursor's own descent depth. A node is finalized
/// (popped and attached to its parent's `children`) the moment the cursor
/// has no more children and no more siblings to explore under it, which is
/// exactly when `metrics::walk_excluding`'s own traversal would have moved
/// on past that subtree.
fn build_ir(
    root: Node,
    language: LanguageFamily,
    source: &str,
    damage_out: &mut Vec<DamageSpan>,
) -> IrNode {
    // `parent_in_catch_body` is the already-computed `in_catch_body` flag of
    // this node's own parent (or `false` for `root`), inherited rather than
    // re-derived -- see `Classification::is_catch_body_root`'s doc comment.
    // `parent` is this node's own parent `Node` (`None` for `root`),
    // threaded down the traversal by the caller for the same reason: a
    // `node.parent()` call restarts at the tree root and descends in
    // tree-sitter 0.25.10, so re-deriving it per node would turn this
    // otherwise-linear tree build into `Θ(n·depth)` work.
    let open = |node: Node,
                parent: Option<Node>,
                parent_in_catch_body: bool,
                damage_out: &mut Vec<DamageSpan>|
     -> (IrNode, bool) {
        let span = Span::from_node(node);
        let classification = classify(node, language, source, parent);
        if let Some(kind) = classification.damage {
            damage_out.push(DamageSpan { kind, span });
        }
        let in_catch_body = classification.is_catch_body_root || parent_in_catch_body;
        let ir_node = IrNode {
            span,
            executable: is_executable_leaf(node, language),
            decision: classification.decision,
            is_terminator: classification.is_terminator,
            in_block: classification.in_block,
            in_catch_body,
            children: Vec::with_capacity(node.child_count()),
        };
        (ir_node, in_catch_body)
    };

    let mut cursor = root.walk();
    let (root_node, root_in_catch_body) = open(root, None, false, damage_out);
    let mut stack: Vec<IrNode> = vec![root_node];
    // Mirrors `stack`'s depth exactly: `catch_flags[i]` is `stack[i]`'s own
    // `in_catch_body` flag, so a child node reads its parent's flag in O(1)
    // via `catch_flags.last()` instead of walking back up the tree.
    let mut catch_flags: Vec<bool> = vec![root_in_catch_body];
    // Mirrors `stack`'s depth exactly too: `parents[i]` is the tree-sitter
    // `Node` that `stack[i]` was itself opened from, so a child node reads
    // its parent's `Node` in O(1) via `parents.last()` instead of via
    // `node.parent()`.
    let mut parents: Vec<Node> = vec![root];
    loop {
        if cursor.goto_first_child() {
            let parent_in_catch_body = *catch_flags.last().unwrap_or(&false);
            let parent = *parents.last().unwrap_or(&root);
            let (node, in_catch_body) = open(
                cursor.node(),
                Some(parent),
                parent_in_catch_body,
                damage_out,
            );
            stack.push(node);
            catch_flags.push(in_catch_body);
            parents.push(cursor.node());
            continue;
        }
        loop {
            let Some(finished) = stack.pop() else {
                return fallback_node(root);
            };
            catch_flags.pop();
            parents.pop();
            let Some(parent) = stack.last_mut() else {
                return finished;
            };
            parent.children.push(finished);
            if cursor.goto_next_sibling() {
                let parent_in_catch_body = *catch_flags.last().unwrap_or(&false);
                let parent_node = *parents.last().unwrap_or(&root);
                let (node, in_catch_body) = open(
                    cursor.node(),
                    Some(parent_node),
                    parent_in_catch_body,
                    damage_out,
                );
                stack.push(node);
                catch_flags.push(in_catch_body);
                parents.push(cursor.node());
                break;
            }
            if !cursor.goto_parent() {
                return fallback_node(root);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tree_sitter::Parser;

    use super::*;
    use crate::clones;
    use crate::model::{Grammar, LanguageFamily};

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    /// Every `.java/.js/.jsx/.mjs/.cjs/.ts/.tsx` file under `tests/fixtures/`,
    /// recursively -- a plain directory walk, not `discover::discover`,
    /// since a gitignore/exclude rule would silently narrow "every fixture"
    /// to less than the acceptance requires.
    fn every_fixture_file(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                every_fixture_file(&path, out);
                continue;
            }
            let extension = path.extension().and_then(|extension| extension.to_str());
            if extension
                .is_some_and(|extension| LanguageFamily::from_extension(extension).is_some())
            {
                out.push(path);
            }
        }
    }

    fn grammar_language(path: &Path) -> Option<(Grammar, LanguageFamily)> {
        let extension = path.extension().and_then(|extension| extension.to_str())?;
        let grammar = Grammar::for_extension(extension)?;
        let language = LanguageFamily::from_extension(extension)?;
        Some((grammar, language))
    }

    fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
        match grammar {
            Grammar::Java => tree_sitter_java::LANGUAGE.into(),
            Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// The IR carries the clone floor (`nsd-plan-final.md` M0b item 4): for
    /// every clone-candidate statement in every in-repo fixture that parses
    /// without error -- the only trees `clones::run` ever sees in
    /// production, via `parse::parse_all`'s own `has_error` filter -- the
    /// lowering's independently-derived token stream is byte-identical to
    /// `clones::normalized_statement_tokens`'s.
    #[test]
    fn test_ir_tokens_reproduce_the_pre_ir_stream_on_every_fixture() {
        let mut files = Vec::new();
        every_fixture_file(&fixtures_root(), &mut files);
        assert!(!files.is_empty(), "expected at least one fixture file");

        let mut compared = 0usize;
        for path in files {
            let Some((grammar, language)) = grammar_language(&path) else {
                continue;
            };
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let mut parser = Parser::new();
            if parser.set_language(&tree_sitter_language(grammar)).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(&source, None) else {
                continue;
            };
            if tree.root_node().has_error() {
                continue;
            }

            let mut statements = Vec::new();
            for_each_descendant(tree.root_node(), |node| {
                if is_clone_statement(node, language) {
                    statements.push(node);
                }
            });

            for statement in statements {
                let ir_tokens = statement_token_stream(statement, language, &source);
                let reference = clones::normalized_statement_tokens(statement, language, &source);
                assert_eq!(
                    ir_tokens,
                    reference,
                    "{}:{} statement token mismatch",
                    path.display(),
                    statement.start_position().row + 1
                );
                compared += 1;
            }
        }
        assert!(compared > 0, "expected at least one comparable statement");
    }

    /// The D11/SLOC requirement: the IR `executable` flag agrees with
    /// `exec_lines::is_executable_leaf` for every node the lowering
    /// produces, across every fixture that parses without error.
    #[test]
    fn test_executable_flag_matches_the_d11_predicate() {
        let mut files = Vec::new();
        every_fixture_file(&fixtures_root(), &mut files);

        let mut checked = 0usize;
        for path in files {
            let Some((grammar, language)) = grammar_language(&path) else {
                continue;
            };
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let mut parser = Parser::new();
            if parser.set_language(&tree_sitter_language(grammar)).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(&source, None) else {
                continue;
            };
            if tree.root_node().has_error() {
                continue;
            }

            let mut damage = Vec::new();
            let root = build_ir(tree.root_node(), language, &source, &mut damage);

            let mut ir_nodes = Vec::new();
            collect_ir_nodes(&root, &mut ir_nodes);
            let mut ts_nodes = Vec::new();
            for_each_descendant(tree.root_node(), |node| ts_nodes.push(node));

            assert_eq!(ir_nodes.len(), ts_nodes.len(), "{}", path.display());
            for (ir_node, ts_node) in ir_nodes.iter().zip(ts_nodes.iter()) {
                let expected = is_executable_leaf(*ts_node, language);
                assert_eq!(
                    ir_node.executable,
                    expected,
                    "{}:{} executable flag mismatch",
                    path.display(),
                    ts_node.start_position().row + 1
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "expected at least one node checked");
    }

    /// Pre-order collect of every `IrNode` in the tree the lowering
    /// produced -- test-only, so a small recursive walk over an already
    /// fully-materialized (and fixture-sized) tree is fine; the lowering
    /// itself never recurses (see `build_ir`'s own doc comment).
    fn collect_ir_nodes<'a>(node: &'a IrNode, out: &mut Vec<&'a IrNode>) {
        out.push(node);
        for child in &node.children {
            collect_ir_nodes(child, out);
        }
    }

    /// Every clone-candidate statement's own token stream, in document
    /// order, for one fixture file under `tests/fixtures/`.
    fn clone_statement_tokens(path: &str, language: LanguageFamily) -> Vec<String> {
        let full_path = fixtures_root().join(path);
        let (grammar, _) = grammar_language(&full_path).expect("known fixture extension");
        let source = fs::read_to_string(&full_path).expect("read fixture");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_language(grammar))
            .expect("set_language");
        let tree = parser.parse(&source, None).expect("parse");
        let mut tokens = Vec::new();
        for_each_descendant(tree.root_node(), |node| {
            if is_clone_statement(node, language) {
                tokens.push(statement_token_stream(node, language, &source));
            }
        });
        tokens
    }

    /// Every clone-candidate statement's own document-order start line, for
    /// one fixture file under `tests/fixtures/` -- test-only support for
    /// `test_clone_candidate_containers_cover_every_container_arm` below.
    fn clone_statement_start_lines(path: &str, language: LanguageFamily) -> Vec<u32> {
        let full_path = fixtures_root().join(path);
        let (grammar, _) = grammar_language(&full_path).expect("known fixture extension");
        let source = fs::read_to_string(&full_path).expect("read fixture");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_language(grammar))
            .expect("set_language");
        let tree = parser.parse(&source, None).expect("parse");
        let mut lines = Vec::new();
        for_each_descendant(tree.root_node(), |node| {
            if is_clone_statement(node, language) {
                lines.push(node.start_position().row as u32 + 1);
            }
        });
        lines
    }

    /// HIGH-2 (round 2): the clone-floor container set was unproven -- the
    /// suite stayed green with `"constructor_body"`/
    /// `"switch_block_statement_group"` (Java) or `"program"`/
    /// `switch_case`/`switch_default` body field (JS/TS) deleted from
    /// `is_clone_statement`. `ContainerSet.java` exercises a constructor
    /// whose `constructor_body` holds exactly two statements and a `switch`
    /// whose `switch_block_statement_group` holds exactly two non-label
    /// statements (plus the `switch_statement` itself, a `block`-parented
    /// clone candidate already covered elsewhere); `container_set.ts`
    /// exercises two top-level `program` statements plus a top-level
    /// function declaration, and a `switch_case` whose `body` field holds
    /// two statements. Asserts document-order start lines, not just a count,
    /// so a swapped container arm (matching the wrong parent kind) cannot
    /// hide behind a coincidentally-equal total.
    #[test]
    fn test_clone_candidate_containers_cover_every_container_arm() {
        let java_lines = clone_statement_start_lines("ir/ContainerSet.java", LanguageFamily::Java);
        assert_eq!(java_lines, vec![3, 4, 8, 10, 11], "{java_lines:?}");

        let ts_lines = clone_statement_start_lines("ir/container_set.ts", LanguageFamily::JsTs);
        assert_eq!(ts_lines, vec![1, 2, 4, 5, 7, 8], "{ts_lines:?}");
    }

    /// The clone-token floor's equality is discriminating, not vacuous
    /// (`nsd-plan-final.md` M0b item 4): two fixtures whose two-statement
    /// clone body agrees on the first statement and differs on the second
    /// produce token streams that agree at index zero and differ at index
    /// one. Calls `statement_token_stream` directly rather than reading
    /// `IrNode::token` -- that field carried no reader in `src/` and was
    /// removed; WS-4 derives a stream on demand the same way this test does.
    #[test]
    fn test_ir_tokens_differ_when_a_statement_differs() {
        let a_tokens = clone_statement_tokens("ir/DifferA.java", LanguageFamily::Java);
        let b_tokens = clone_statement_tokens("ir/DifferB.java", LanguageFamily::Java);

        assert_eq!(a_tokens.len(), 2, "{a_tokens:?}");
        assert_eq!(b_tokens.len(), 2, "{b_tokens:?}");
        assert_eq!(a_tokens[0], b_tokens[0]);
        assert_ne!(a_tokens[1], b_tokens[1]);
    }
}

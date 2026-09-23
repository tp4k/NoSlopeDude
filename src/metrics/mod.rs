//! Stage 2 (WS-2): CC and erosion metrics. Walks each parsed file's tree,
//! in parallel across files (D21), to find callables (D8), computes each
//! callable's CC from the documented decision constructs (D7,
//! `docs/cc-rules.md`) and its SLOC from grammar-derived named tokens
//! (D11), publishes mass and erosion (D13), and ranks the top 25 callables
//! by CC descending. Every tree walk here is an iterative `TreeCursor`
//! traversal (`walk_excluding`) rather than per-AST-depth recursion, so an
//! arbitrarily deep source tree degrades in time, not by aborting (D18).

use std::cmp::Ordering;

use rayon::prelude::*;
use tree_sitter::{Node, TreeCursor};

use crate::exec_lines::is_executable_leaf;
use crate::ir::{DecisionKind, IrNode, Span};
use crate::lower;
use crate::model::{
    Callable, FileScanSummary, LanguageFamily, MetricsResult, SyntaxBlock, CC_EROSION_THRESHOLD,
};
use crate::parse::ParsedFile;

/// How many rows the top-callables ranking keeps.
const TOP_CALLABLES: usize = 25;

const JAVA_CALLABLE_KINDS: &[&str] = &[
    "method_declaration",
    "constructor_declaration",
    "compact_constructor_declaration",
    "static_initializer",
    "lambda_expression",
];

const JSTS_CALLABLE_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "arrow_function",
    "method_definition",
];

/// Runs the metrics stage over every successfully parsed file, one rayon
/// task per file (D21) since `parse_all` already fans out the same way and
/// this walk is the larger half of the work. `incomplete` marks the result
/// D18-incomplete when at least one file failed to parse.
pub fn run(parsed_files: &[ParsedFile], incomplete: bool) -> MetricsResult {
    let per_file: Vec<(Vec<Callable>, Vec<SyntaxBlock>, FileScanSummary)> =
        parsed_files.par_iter().map(scan_file).collect();

    let mut callables = Vec::new();
    let mut syntax_blocks = Vec::new();
    let mut file_scan_summaries = Vec::with_capacity(per_file.len());
    for (file_callables, file_blocks, summary) in per_file {
        callables.extend(file_callables);
        syntax_blocks.extend(file_blocks);
        file_scan_summaries.push(summary);
    }

    callables.sort_unstable_by(|a, b| {
        a.relative_path
            .cmp(&b.relative_path)
            .then(a.start_line.cmp(&b.start_line))
    });

    let erosion = erosion(&callables);
    let top25 = rank_top_callables(&callables);

    MetricsResult {
        callables,
        syntax_blocks,
        file_scan_summaries,
        erosion,
        top25,
        incomplete,
    }
}

/// D13: `Σ mass where cc > CC_EROSION_THRESHOLD / Σ mass`, `0.0` when the
/// total mass is `0.0` (including when `callables` is empty).
pub fn erosion(callables: &[Callable]) -> f64 {
    let total_mass: f64 = callables.iter().map(|callable| callable.mass).sum();
    if total_mass == 0.0 {
        return 0.0;
    }
    let eroded_mass: f64 = callables
        .iter()
        .filter(|callable| callable.cc > CC_EROSION_THRESHOLD)
        .map(|callable| callable.mass)
        .sum();
    eroded_mass / total_mass
}

/// D13: `mass = cc as f64 * (sloc as f64).sqrt()`, verbatim.
pub fn mass(cc: u32, sloc: usize) -> f64 {
    cc as f64 * (sloc as f64).sqrt()
}

/// Total order for the top-callables ranking: `cc` descending, ties broken
/// by path then start line, so the output is deterministic.
fn cmp_by_cc_desc_then_location(a: &&Callable, b: &&Callable) -> Ordering {
    b.cc.cmp(&a.cc)
        .then_with(|| a.relative_path.cmp(&b.relative_path))
        .then_with(|| a.start_line.cmp(&b.start_line))
}

/// Top `TOP_CALLABLES` callables by `cc` descending (see
/// `cmp_by_cc_desc_then_location`). Selects the top slice in O(n) via
/// `select_nth_unstable_by`, sorts and clones only that slice, instead of
/// sorting and deep-cloning every callable just to keep 25 rows.
pub fn rank_top_callables(callables: &[Callable]) -> Vec<Callable> {
    let mut refs: Vec<&Callable> = callables.iter().collect();
    if refs.is_empty() {
        return Vec::new();
    }
    let keep = TOP_CALLABLES.min(refs.len());
    refs.select_nth_unstable_by(keep - 1, cmp_by_cc_desc_then_location);
    let top = &mut refs[..keep];
    top.sort_unstable_by(cmp_by_cc_desc_then_location);
    top.iter().map(|callable| (**callable).clone()).collect()
}

fn callable_kinds(language: LanguageFamily) -> &'static [&'static str] {
    match language {
        LanguageFamily::Java => JAVA_CALLABLE_KINDS,
        LanguageFamily::JsTs => JSTS_CALLABLE_KINDS,
    }
}

/// D8: a callable-kind node that has a body. `function_signature`,
/// `abstract_method_signature` and similar bodyless kinds are never in
/// `callable_kinds`, so they are excluded by the kind check alone; a
/// bodyless Java `method_declaration` (interface or abstract method) is
/// excluded by the body check.
fn is_callable_node(node: Node, language: LanguageFamily) -> bool {
    callable_kinds(language).contains(&node.kind()) && callable_body(node, language).is_some()
}

/// The callable's body node. Every callable kind exposes it through the
/// `body` field, except Java's `static_initializer`, whose direct `block`
/// child carries no field name.
fn callable_body<'tree>(node: Node<'tree>, _language: LanguageFamily) -> Option<Node<'tree>> {
    node.child_by_field_name("body")
        .or_else(|| first_child_of_kind(node, "block"))
}

/// One pass over `file`'s whole tree: finds every callable (spawning
/// `scan_callable_body`'s own pass for its cc/SLOC, D9), collects every
/// block-kind node as a `SyntaxBlock` — at module level and inside
/// callables alike, so a module-level block is no longer silently
/// dropped — and accumulates the file's D12 scanned-line count, all from
/// the same `TreeCursor`.
fn scan_file(file: &ParsedFile) -> (Vec<Callable>, Vec<SyntaxBlock>, FileScanSummary) {
    // Lowered once per file: every callable's own cc/SLOC walk below reads
    // its subtree off this same tree instead of tree-sitter nodes.
    let ir_file = lower::lower_file(file);

    let mut callables = Vec::new();
    let mut syntax_blocks = Vec::new();
    let mut scanned_lines = 0usize;
    let mut last_counted_line = 0usize;

    walk_excluding(
        file.tree.root_node(),
        |_node| false,
        |node| {
            if is_executable_leaf(node, file.language) {
                accumulate_line(node, &mut scanned_lines, &mut last_counted_line);
            }
            if is_block_kind(node.kind(), file.language) {
                syntax_blocks.push(SyntaxBlock {
                    relative_path: file.relative_path.clone(),
                    language: file.language,
                    kind: node.kind(),
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                });
            }
            if is_callable_node(node, file.language) {
                // `is_callable_node` already confirmed a body is present.
                if let Some(body) = callable_body(node, file.language) {
                    let CallableMetrics { cc, sloc } =
                        scan_callable_body(body, file.language, &ir_file.root);
                    callables.push(Callable {
                        relative_path: file.relative_path.clone(),
                        language: file.language,
                        name: resolve_name(node, &file.source),
                        start_line: node.start_position().row + 1,
                        cc,
                        sloc,
                        mass: mass(cc, sloc),
                    });
                }
            }
        },
    );

    let summary = FileScanSummary {
        relative_path: file.relative_path.clone(),
        scanned_lines,
    };
    (callables, syntax_blocks, summary)
}

struct CallableMetrics {
    cc: u32,
    sloc: usize,
}

/// One callable body's cc/SLOC, walking `body`'s own `IrNode` subtree
/// (found in `ir_root`, the whole file's lowered tree) rather than
/// tree-sitter nodes directly -- `is_callable_node`/`callable_body` stay the
/// tree-sitter-based D8 boundary finders they always were; only the
/// decision/executable accumulation they hand off to is retargeted.
fn scan_callable_body(body: Node, language: LanguageFamily, ir_root: &IrNode) -> CallableMetrics {
    if is_callable_node(body, language) {
        // The callable's own body is itself a callable (e.g. a curried
        // `(a) => (b) => …`): D9 excludes it entirely, matching
        // `walk_excluding`'s own `exclude(root)` check.
        return CallableMetrics { cc: 1, sloc: 0 };
    }

    let body_span = Span::from_node(body);
    let fallback = fallback_ir_body(body_span);
    let body_ir = find_ir_subtree(ir_root, body_span).unwrap_or(&fallback);
    let excluded = collect_nested_callable_spans(body, language);

    let mut cc = 1u32;
    let mut sloc = 0usize;
    let mut last_counted_line = 0usize;

    walk_ir_excluding(body_ir, &excluded, |node| {
        if let Some(kind) = node.decision {
            cc += decision_weight(kind);
        }
        if node.executable {
            accumulate_ir_line(node.span, &mut sloc, &mut last_counted_line);
        }
    });

    CallableMetrics { cc, sloc }
}

/// The byte spans of every top-level nested callable inside `body` (D9):
/// found via `is_callable_node`, the same tree-sitter predicate `scan_file`
/// itself uses -- the descent stops the moment one is found, since anything
/// further inside it is already covered by its own span. Iterative (an
/// explicit work-list, no recursion), so a body nested deep inside a
/// pathological source tree cannot overflow the stack.
fn collect_nested_callable_spans(body: Node, language: LanguageFamily) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut pending = vec![body];
    while let Some(node) = pending.pop() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_callable_node(child, language) {
                spans.push(Span::from_node(child));
            } else {
                pending.push(child);
            }
        }
    }
    spans
}

/// D9's own exclusion, over the IR: visits every `IrNode` under `root` in
/// document order, skipping any subtree whose span falls inside one of
/// `excluded` (a nested callable's own span). Iterative -- see
/// `collect_nested_callable_spans`'s own doc comment for why.
fn walk_ir_excluding<'a>(root: &'a IrNode, excluded: &[Span], mut visit: impl FnMut(&'a IrNode)) {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if is_span_excluded(node.span, excluded) {
            continue;
        }
        visit(node);
        for child in node.children.iter().rev() {
            pending.push(child);
        }
    }
}

fn is_span_excluded(span: Span, excluded: &[Span]) -> bool {
    excluded
        .iter()
        .any(|excl| excl.start_byte <= span.start_byte && span.end_byte <= excl.end_byte)
}

/// Locates the `IrNode` produced for the tree-sitter node whose span is
/// `target`, by descending from `root` through whichever child's span
/// contains it -- valid since `build_ir` (`src/lower/mod.rs`) produces one
/// `IrNode` per tree-sitter node in the same nested-span shape. A `while`
/// loop, not recursion, so a deeply nested callable body cannot overflow the
/// stack.
fn find_ir_subtree(root: &IrNode, target: Span) -> Option<&IrNode> {
    let mut current = root;
    while current.span != target {
        current = current.children.iter().find(|child| {
            child.span.start_byte <= target.start_byte && target.end_byte <= child.span.end_byte
        })?;
    }
    Some(current)
}

/// A structurally-unreachable fallback for `find_ir_subtree` returning
/// `None`: `build_ir` produces exactly one `IrNode` per tree-sitter node, in
/// the same nested-span shape, so a callable body's own span always has a
/// match. Kept only so a defect here degrades (D18) instead of panicking --
/// same reasoning as `lower::fallback_node`'s own doc comment.
fn fallback_ir_body(span: Span) -> IrNode {
    IrNode {
        span,
        executable: false,
        decision: None,
        is_terminator: false,
        in_block: false,
        in_catch_body: false,
        children: Vec::new(),
    }
}

/// `accumulate_line`'s own logic (see its doc comment), re-derived for a
/// `Span` (the IR) instead of a tree-sitter `Node` -- `accumulate_line`
/// itself is unchanged, still used by `scan_file`'s own tree-sitter-only D12
/// count.
fn accumulate_ir_line(span: Span, count: &mut usize, last_counted_line: &mut usize) {
    let start = span.start_line as usize;
    let end = span.end_line as usize;
    let from = start.max(*last_counted_line + 1);
    if from <= end {
        *count += end - from + 1;
        *last_counted_line = end;
    }
}

/// Iterative pre-order traversal via a single reused `TreeCursor` (no
/// native call-stack growth, unlike per-AST-depth-level recursion): visits
/// `root` and every descendant, except a subtree rooted at a node `exclude`
/// accepts is skipped entirely (used for D9: a nested callable's span).
fn walk_excluding<'tree>(
    root: Node<'tree>,
    mut exclude: impl FnMut(Node<'tree>) -> bool,
    mut visit: impl FnMut(Node<'tree>),
) {
    if exclude(root) {
        return;
    }
    let mut cursor = root.walk();
    loop {
        visit(cursor.node());
        if descend_to_included_child(&mut cursor, &mut exclude) {
            continue;
        }
        if !advance_to_included_sibling(&mut cursor, &mut exclude) {
            return;
        }
    }
}

/// Moves `cursor` to its first child `exclude` accepts, skipping past any
/// excluded children (and their subtrees) entirely. Restores `cursor` to
/// the parent node if no child qualifies.
fn descend_to_included_child<'tree>(
    cursor: &mut TreeCursor<'tree>,
    exclude: &mut impl FnMut(Node<'tree>) -> bool,
) -> bool {
    if !cursor.goto_first_child() {
        return false;
    }
    loop {
        if !exclude(cursor.node()) {
            return true;
        }
        if !cursor.goto_next_sibling() {
            cursor.goto_parent();
            return false;
        }
    }
}

/// Moves `cursor` to the next sibling `exclude` accepts, walking up through
/// parents as each level is exhausted. A `TreeCursor` refuses to walk above
/// the node it was constructed with, so this naturally stops at `root`.
fn advance_to_included_sibling<'tree>(
    cursor: &mut TreeCursor<'tree>,
    exclude: &mut impl FnMut(Node<'tree>) -> bool,
) -> bool {
    loop {
        if cursor.goto_next_sibling() {
            if !exclude(cursor.node()) {
                return true;
            }
            continue;
        }
        if !cursor.goto_parent() {
            return false;
        }
    }
}

/// Adds the leaf's not-yet-counted lines to `count`, relying on
/// `walk_excluding` visiting leaves in non-decreasing source-line order —
/// so a running "highest line already counted" replaces a per-callable/
/// per-file `HashSet<usize>` of every line seen.
fn accumulate_line(node: Node, count: &mut usize, last_counted_line: &mut usize) {
    let start = node.start_position().row + 1;
    let end = node.end_position().row + 1;
    let from = start.max(*last_counted_line + 1);
    if from <= end {
        *count += end - from + 1;
        *last_counted_line = end;
    }
}

/// D10: the node's own `name` field; else the name from an enclosing
/// `variable_declarator`, `pair` or `assignment_expression`; else
/// `<anonymous>@<line>`.
fn resolve_name(node: Node, source: &str) -> String {
    if let Some(name_node) = node.child_by_field_name("name") {
        return node_text(name_node, source);
    }
    if let Some(parent) = node.parent() {
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

/// `cc`'s weight table (*What the IR must carry*), keyed on the IR's
/// uniform `DecisionKind` rather than per-grammar strings: every counted
/// decision point adds 1, matching the pre-IR match arms' own weights
/// exactly (`decision` is `None` for `else`/`finally`/a default `switch`
/// label, so those already contribute 0 without an arm here).
fn decision_weight(kind: DecisionKind) -> u32 {
    match kind {
        DecisionKind::Branch
        | DecisionKind::Loop
        | DecisionKind::Case
        | DecisionKind::Catch
        | DecisionKind::Ternary
        | DecisionKind::And
        | DecisionKind::Or => 1,
    }
}

fn first_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|child| child.kind() == kind);
    found
}

/// Every `{ … }` scope block in a file — module level and inside a
/// callable alike — kept for WS-3's duplicate-block detection.
fn is_block_kind(kind: &str, language: LanguageFamily) -> bool {
    match language {
        LanguageFamily::Java => matches!(kind, "block" | "constructor_body"),
        LanguageFamily::JsTs => kind == "statement_block",
    }
}

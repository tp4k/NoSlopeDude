//! Stage 2 (WS-2/WS-3): CC and erosion metrics. Each parsed file is lowered
//! once (`lower::lower_file`), and every callable (D8), block (`SyntaxBlock`)
//! and SLOC count (D11/D12) comes off that `IrFile` rather than a
//! tree-sitter node: `IrFile::callables` already carries D8's declaration/
//! body boundaries and D10's resolved name, `IrFile::blocks` already carries
//! the self-is-block classification, and every `IrNode` already carries its
//! own `executable`/`decision` flags. Computes each callable's CC from the
//! documented decision constructs (D7, `docs/cc-rules.md`) and its SLOC from
//! the IR's `executable` flag, publishes mass and erosion (D13), and ranks
//! the top 25 callables by CC descending. Every IR walk here
//! (`walk_ir_excluding`) is iterative, not per-AST-depth recursion, so an
//! arbitrarily deep source tree degrades in time, not by aborting (D18).

use std::cmp::Ordering;
use std::path::Path;

use rayon::prelude::*;

use crate::ir::{DecisionKind, IrCallable, IrNode, Span};
use crate::lower;
use crate::model::{
    Callable, FileScanSummary, LanguageFamily, MetricsResult, SyntaxBlock, CC_EROSION_THRESHOLD,
};
use crate::parse::ParsedFile;

/// How many rows the top-callables ranking keeps.
const TOP_CALLABLES: usize = 25;

/// Runs the metrics stage over every successfully parsed file. Lowers each
/// file itself (`lower::lower_all`) and delegates to `run_with_ir` below --
/// a thin wrapper kept so this signature's existing call sites (mostly
/// tests) stay untouched. `incomplete` marks the result D18-incomplete
/// when at least one file failed to parse.
pub fn run(parsed_files: &[ParsedFile], incomplete: bool) -> MetricsResult {
    let (ir_files, unanalyzed_lines) = lower::lower_all_inventoried(parsed_files);
    run_with_ir(parsed_files, &ir_files, &unanalyzed_lines, incomplete)
}

/// WS-9 (C1): identical to `run` above, but takes the pipeline's own
/// single lowering pass instead of lowering `parsed_files` again --
/// `pipeline::run`'s production path calls this directly, `ir_files`
/// index-aligned with `parsed_files` (`lower::lower_all`'s own
/// `par_iter` preserves order). One rayon task per file (D21) since
/// `parse_all` already fans out the same way and this walk is the larger
/// half of the work.
pub(crate) fn run_with_ir(
    parsed_files: &[ParsedFile],
    ir_files: &[lower::IrFile],
    unanalyzed_lines: &[usize],
    incomplete: bool,
) -> MetricsResult {
    debug_assert_eq!(parsed_files.len(), ir_files.len());
    debug_assert_eq!(parsed_files.len(), unanalyzed_lines.len());
    let per_file: Vec<(Vec<Callable>, Vec<SyntaxBlock>, FileScanSummary)> = parsed_files
        .par_iter()
        .zip(ir_files.par_iter())
        .zip(unanalyzed_lines.par_iter())
        .map(|((file, ir_file), unanalyzed)| scan_file(file, ir_file, *unanalyzed))
        .collect();

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
    if eroded_mass == 0.0 {
        // `Iterator::sum::<f64>()` over an empty (or empty-after-filter)
        // sequence is `-0.0` on this toolchain; force the positive literal
        // so a caller sees `+0.0`, not `-0.0`, when nothing is eroded.
        return 0.0;
    }
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

/// One pass over `file`'s lowered IR: turns `IrFile::callables` into
/// `Callable`s (spawning `scan_callable_body`'s own walk for each one's
/// cc/SLOC, D9) and `IrFile::blocks` into `SyntaxBlock`s — at module level
/// and inside callables alike, both tables already in document order — and
/// accumulates the file's D12 scanned-line count over the whole IR tree,
/// unexcluded. WS-9 (C1): `ir_file` is the caller's own lowering
/// (`run_with_ir`'s `ir_files`, index-aligned with `parsed_files`), not
/// lowered again here.
fn scan_file(
    file: &ParsedFile,
    ir_file: &lower::IrFile,
    unanalyzed_lines: usize,
) -> (Vec<Callable>, Vec<SyntaxBlock>, FileScanSummary) {
    let mut scanned_lines = 0usize;
    let mut last_counted_line = 0usize;
    walk_ir_excluding(&ir_file.root, &[], |node| {
        if node.executable {
            accumulate_ir_line(node.span, &mut scanned_lines, &mut last_counted_line);
        }
    });

    let syntax_blocks = ir_file
        .blocks
        .iter()
        .map(|block| SyntaxBlock {
            relative_path: file.relative_path.clone(),
            language: file.language,
            kind: block.kind,
            start_line: block.span.start_line as usize,
            end_line: block.span.end_line as usize,
        })
        .collect();

    let callables = callables_from_ir(&file.relative_path, file.language, ir_file);

    let summary = FileScanSummary {
        relative_path: file.relative_path.clone(),
        scanned_lines,
        unanalyzed_lines,
        gaps: Vec::new(),
        unmeasured_callables: 0,
    };
    (callables, syntax_blocks, summary)
}

/// One `Callable` per `IrFile::callables` entry, index-aligned with it.
pub(crate) fn callables_from_ir(
    relative_path: &Path,
    language: LanguageFamily,
    ir_file: &lower::IrFile,
) -> Vec<Callable> {
    ir_file
        .callables
        .iter()
        .map(|callable| {
            let CallableMetrics { cc, sloc } = scan_callable_body(callable.body_span, ir_file);
            Callable {
                relative_path: relative_path.to_path_buf(),
                language,
                name: callable.name.clone(),
                start_line: callable.span.start_line as usize,
                end_line: callable.span.end_line as usize,
                cc,
                sloc,
                mass: mass(cc, sloc),
            }
        })
        .collect()
}

struct CallableMetrics {
    cc: u32,
    sloc: usize,
}

/// One callable body's cc/SLOC, walking `body_span`'s own `IrNode` subtree
/// (found in `ir_file.root`) rather than tree-sitter nodes -- D8's
/// declaration/body boundaries and D10's name now come off
/// `IrFile::callables` (`scan_file` iterates it directly), not a
/// tree-sitter descent from this function.
fn scan_callable_body(body_span: Span, ir_file: &lower::IrFile) -> CallableMetrics {
    if body_is_nested_callable(body_span, &ir_file.callables) {
        // The callable's own body is itself a callable (e.g. a curried
        // `(a) => (b) => …`): D9 excludes it entirely, matching
        // `walk_ir_excluding`'s own exclusion of a nested callable's span.
        return CallableMetrics { cc: 1, sloc: 0 };
    }

    let fallback = fallback_ir_body(body_span);
    let body_ir = find_ir_subtree(&ir_file.root, body_span).unwrap_or(&fallback);
    let excluded = nested_callable_spans(body_span, &ir_file.callables);

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

/// D9's curried-body special case: the callable's own body node is itself
/// another callable's declaration (e.g. `(a) => (b) => …`, no wrapping
/// block), true exactly when some callable in the same file's
/// `IrFile::callables` declares itself at `body_span`.
fn body_is_nested_callable(body_span: Span, callables: &[IrCallable]) -> bool {
    callables.iter().any(|callable| callable.span == body_span)
}

/// D9's own exclusion set for one callable's body: every other callable in
/// the same file (`IrFile::callables`) whose own declaration span sits
/// inside `body_span` -- in place of the tree-sitter descent
/// `collect_nested_callable_spans` used to perform for the same purpose.
fn nested_callable_spans(body_span: Span, callables: &[IrCallable]) -> Vec<Span> {
    callables
        .iter()
        .filter(|callable| {
            callable.span != body_span
                && body_span.start_byte <= callable.span.start_byte
                && callable.span.end_byte <= body_span.end_byte
        })
        .map(|callable| callable.span)
        .collect()
}

/// Iterative pre-order traversal via an explicit work-list (no native
/// call-stack growth, unlike per-AST-depth-level recursion): visits `root`
/// and every descendant, except a subtree whose span falls inside one of
/// `excluded` is skipped entirely (used for D9: a nested callable's span).
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

/// Fallback for `find_ir_subtree` returning `None` -- no `IrNode` in the
/// redacted tree matches a callable's body span. WS-6 round 4's fix to
/// `cascade_exclusions` (excluding an entity contained in a bare damage
/// span) closed the one known input shape that reached this branch; this
/// crate's own suite hits it zero times, as did a 400-input fuzz run at
/// WS-6 round 4 (before M0c's grammar swap), but unlike
/// `lower::fallback_node`'s structural unreachability, that is evidence,
/// not a proof of unreachability. Kept so a future gap here degrades (D18)
/// instead of panicking.
fn fallback_ir_body(span: Span) -> IrNode {
    IrNode::empty(span)
}

/// Adds the leaf's not-yet-counted lines to `count`, relying on
/// `walk_ir_excluding` visiting nodes in non-decreasing source-line order —
/// so a running "highest line already counted" replaces a per-callable/
/// per-file `HashSet<usize>` of every line seen.
fn accumulate_ir_line(span: Span, count: &mut usize, last_counted_line: &mut usize) {
    let start = span.start_line as usize;
    let end = span.end_line as usize;
    let from = start.max(*last_counted_line + 1);
    if from <= end {
        *count += end - from + 1;
        *last_counted_line = end;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const SOURCE: &str = "class C {\n    int m(int a) {\n        if (a > 0) {\n            return 1;\n        }\n        return 0;\n    }\n}\n";

    fn lowered(source: &str) -> lower::IrFile {
        let (parsed, _) = parse::parse_source(
            Path::new("C.java"),
            LanguageFamily::Java,
            source.to_string(),
        );
        lower::lower_file(&parsed.expect("java parses"))
    }

    fn span(start_byte: u32, end_byte: u32, start_line: u32, end_line: u32) -> Span {
        Span {
            start_byte,
            end_byte,
            start_line,
            end_line,
        }
    }

    /// Deferred row 51: a body span with no IR subtree measures as an empty
    /// body, whichever span it is (past the source, or inside one token).
    #[test]
    fn test_a_body_without_an_ir_subtree_measures_cc_1_sloc_0() {
        let ir_file = lowered(SOURCE);
        let unmatched = [
            span(10_000, 10_040, 400, 410),
            span(1, 3, 1, 1),
            span(u32::MAX - 1, u32::MAX, 1, 1),
        ];
        for body_span in unmatched {
            assert!(find_ir_subtree(&ir_file.root, body_span).is_none());
            let CallableMetrics { cc, sloc } = scan_callable_body(body_span, &ir_file);
            assert_eq!((cc, sloc), (1, 0), "{body_span:?}");
        }
    }
}

//! Stage 4 (WS-4): wasteful-code rules and the adapted verbosity score
//! (D22, D23). Six conservative, syntax-only rules — documented with their
//! conservative-by-design rationale in `docs/wasteful-rules.md` — run over
//! the IR (`nsd-plan-final.md` M0b item 6): terminator-ness, block
//! membership and catch-body membership are IR structural queries here, not
//! grammar-string matches. This is an adaptation of scb-check's Python-only
//! wasteful-code rules, not a port and not a claim of numerical
//! equivalence — see `docs/wasteful-rules.md`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;

use crate::clones::redundant_occurrences;
use crate::ir::{self, DecisionKind, IrNode};
use crate::lower;
use crate::model::{
    CloneGroup, ClonesResult, FileLanguageLines, LanguageFamily, MetricsResult, RuleFinding,
    RuleId, RulesResult, VerbosityScore, VerbosityScores,
};
use crate::parse::ParsedFile;

pub const JAVA_UNREACHABLE_AFTER_RETURN: RuleId = "JAVA-UNREACHABLE-AFTER-RETURN";
pub const JAVA_EMPTY_CATCH: RuleId = "JAVA-EMPTY-CATCH";
pub const JAVA_REDUNDANT_ELSE_AFTER_RETURN: RuleId = "JAVA-REDUNDANT-ELSE-AFTER-RETURN";
pub const JSTS_UNREACHABLE_AFTER_RETURN: RuleId = "JSTS-UNREACHABLE-AFTER-RETURN";
pub const JSTS_EMPTY_CATCH: RuleId = "JSTS-EMPTY-CATCH";
pub const JSTS_REDUNDANT_ELSE_AFTER_RETURN: RuleId = "JSTS-REDUNDANT-ELSE-AFTER-RETURN";

/// D22's six v1 rule ids, closed for v1.
/// `tests/rules.rs::test_every_rule_id_is_documented` checks each one
/// appears in `docs/wasteful-rules.md`.
pub const ALL_RULE_IDS: [RuleId; 6] = [
    JAVA_UNREACHABLE_AFTER_RETURN,
    JAVA_EMPTY_CATCH,
    JAVA_REDUNDANT_ELSE_AFTER_RETURN,
    JSTS_UNREACHABLE_AFTER_RETURN,
    JSTS_EMPTY_CATCH,
    JSTS_REDUNDANT_ELSE_AFTER_RETURN,
];

/// Runs the rules stage: every rule finding (D22) plus the D23 verbosity
/// score, overall and per language family (D20).
pub fn run(
    parsed_files: &[ParsedFile],
    metrics: &MetricsResult,
    clones: &ClonesResult,
) -> RulesResult {
    let findings = find_findings(parsed_files);
    let files = file_language_lines(parsed_files, metrics);
    let verbosity = compute_verbosity(&files, &findings, &clones.groups);
    RulesResult {
        findings,
        verbosity,
        incomplete: metrics.incomplete,
    }
}

/// D22: every rule finding across every parsed file, one rayon task per
/// file (D21), sorted for a deterministic result.
pub fn find_findings(parsed_files: &[ParsedFile]) -> Vec<RuleFinding> {
    let per_file: Vec<Vec<RuleFinding>> =
        parsed_files.par_iter().map(scan_file_for_rules).collect();
    let mut findings = Vec::new();
    for file_findings in per_file {
        findings.extend(file_findings);
    }
    findings.sort_by(|a, b| {
        a.relative_path
            .cmp(&b.relative_path)
            .then(a.start_line.cmp(&b.start_line))
            .then(a.rule_id.cmp(b.rule_id))
    });
    findings
}

/// D12/D20/D11: joins WS-2's per-file scanned-line count with the language
/// and D11-filtered executable-line set the IR carries for that same file --
/// `FileScanSummary` alone has neither. The lowering this does per file
/// (`lower::lower_file`) is the only whole-tree build on this path, so it
/// fans out with rayon (D21) like every other whole-corpus pass in the
/// pipeline; `files_by_path` is built serially first and shared by
/// reference (`ParsedFile` is already proven `Sync` by `find_findings`'s own
/// `par_iter` above). Rayon's `collect` into a `Vec` preserves input order
/// (`file_scan_summaries`'s order), so `files`'s order and every downstream
/// index (`path_index` in `compute_verbosity`) are unchanged.
fn file_language_lines(
    parsed_files: &[ParsedFile],
    metrics: &MetricsResult,
) -> Vec<FileLanguageLines> {
    let files_by_path: HashMap<&Path, &ParsedFile> = parsed_files
        .iter()
        .map(|file| (file.relative_path.as_path(), file))
        .collect();
    metrics
        .file_scan_summaries
        .par_iter()
        .filter_map(|summary| {
            files_by_path
                .get(summary.relative_path.as_path())
                .map(|file| {
                    let ir_file = lower::lower_file(file);
                    let mut executable_lines = BTreeSet::new();
                    collect_ir_executable_lines(&ir_file.root, &mut executable_lines);
                    FileLanguageLines {
                        relative_path: summary.relative_path.clone(),
                        language: file.language,
                        scanned_lines: summary.scanned_lines,
                        executable_lines,
                    }
                })
        })
        .collect()
}

/// D23: the verbosity aggregation, decoupled from AST access so it can be
/// tested directly against hand-built findings and clone groups. A rule
/// finding contributes its own `flagged_lines` (already D11-filtered at
/// detection time, see `collect_ir_executable_lines`); a clone group
/// contributes, for every occurrence `redundant_occurrences` returns (every
/// occurrence but the canonically first, D14), only the lines within its
/// `[start_line, end_line]` span that are also members of that file's own
/// `executable_lines` (D11) — a blank, comment-only or brace-only line
/// inside an occurrence's span does not inflate the numerator, matching how
/// a rule finding's own `flagged_lines` are already filtered. The two
/// contributions are kept in one `HashSet<usize>` per file — a
/// `Vec<HashSet<usize>>` indexed by each file's position in `files`, not a
/// `HashSet<(PathBuf, usize)>` — so a finding's or occurrence's path is
/// resolved to that index once, not cloned once per flagged line. `overall`
/// sums every set's length; the per-family numerators sum the same lengths
/// filtered on `files[i].language`, so a line flagged by both a rule and a
/// clone still counts once.
pub fn compute_verbosity(
    files: &[FileLanguageLines],
    findings: &[RuleFinding],
    clone_groups: &[CloneGroup],
) -> VerbosityScores {
    let path_index: HashMap<&Path, u32> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.relative_path.as_path(), index as u32))
        .collect();

    let mut scanned_overall = 0usize;
    let mut scanned_java = 0usize;
    let mut scanned_js_ts = 0usize;
    for file in files {
        scanned_overall += file.scanned_lines;
        match file.language {
            LanguageFamily::Java => scanned_java += file.scanned_lines,
            LanguageFamily::JsTs => scanned_js_ts += file.scanned_lines,
        }
    }

    let mut flagged: Vec<HashSet<usize>> = vec![HashSet::new(); files.len()];
    for finding in findings {
        if let Some(&index) = path_index.get(finding.relative_path.as_path()) {
            flagged[index as usize].extend(finding.flagged_lines.iter().copied());
        }
    }
    for group in clone_groups {
        for occurrence in redundant_occurrences(group) {
            let Some(&index) = path_index.get(occurrence.relative_path.as_path()) else {
                continue;
            };
            let executable_lines = &files[index as usize].executable_lines;
            let lines_in_span = (occurrence.start_line..=occurrence.end_line)
                .filter(|line| executable_lines.contains(line));
            flagged[index as usize].extend(lines_in_span);
        }
    }

    let overall: usize = flagged.iter().map(HashSet::len).sum();
    let flagged_java: usize = flagged
        .iter()
        .zip(files)
        .filter(|(_, file)| file.language == LanguageFamily::Java)
        .map(|(set, _)| set.len())
        .sum();
    let flagged_js_ts: usize = flagged
        .iter()
        .zip(files)
        .filter(|(_, file)| file.language == LanguageFamily::JsTs)
        .map(|(set, _)| set.len())
        .sum();

    VerbosityScores {
        overall: verbosity_score(overall, scanned_overall),
        java: verbosity_score(flagged_java, scanned_java),
        js_ts: verbosity_score(flagged_js_ts, scanned_js_ts),
    }
}

/// D23: `0.0` when `scanned_lines` is `0`, including an empty scan.
fn verbosity_score(flagged_lines: usize, scanned_lines: usize) -> VerbosityScore {
    let ratio = if scanned_lines == 0 {
        0.0
    } else {
        flagged_lines as f64 / scanned_lines as f64
    };
    VerbosityScore {
        flagged_lines,
        scanned_lines,
        ratio,
    }
}

/// D22: every rule finding in one parsed file, from a single pass over its
/// lowered IR tree (no D9 nested-callable exclusion — a rule applies inside
/// a nested callable's body too).
fn scan_file_for_rules(file: &ParsedFile) -> Vec<RuleFinding> {
    let ir_file = lower::lower_file(file);
    let language = file.language;
    let mut findings = Vec::new();
    for_each_ir_node(&ir_file.root, |node, is_root| {
        if is_unreachable_container(node, is_root, language) {
            if let Some((start_line, end_line, flagged_lines)) = find_unreachable_after_return(node)
            {
                findings.push(RuleFinding {
                    relative_path: file.relative_path.clone(),
                    language,
                    rule_id: unreachable_rule_id(language),
                    start_line,
                    end_line,
                    flagged_lines,
                });
            }
        }
        if node.decision == Some(DecisionKind::Catch) {
            if let Some((start_line, end_line, flagged_lines)) = find_empty_catch(node) {
                findings.push(RuleFinding {
                    relative_path: file.relative_path.clone(),
                    language,
                    rule_id: empty_catch_rule_id(language),
                    start_line,
                    end_line,
                    flagged_lines,
                });
            }
        }
        if node.decision == Some(DecisionKind::Branch) {
            if let Some((start_line, end_line, flagged_lines)) = find_redundant_else(node) {
                findings.push(RuleFinding {
                    relative_path: file.relative_path.clone(),
                    language,
                    rule_id: redundant_else_rule_id(language),
                    start_line,
                    end_line,
                    flagged_lines,
                });
            }
        }
    });
    findings
}

fn unreachable_rule_id(language: LanguageFamily) -> RuleId {
    match language {
        LanguageFamily::Java => JAVA_UNREACHABLE_AFTER_RETURN,
        LanguageFamily::JsTs => JSTS_UNREACHABLE_AFTER_RETURN,
    }
}

fn empty_catch_rule_id(language: LanguageFamily) -> RuleId {
    match language {
        LanguageFamily::Java => JAVA_EMPTY_CATCH,
        LanguageFamily::JsTs => JSTS_EMPTY_CATCH,
    }
}

fn redundant_else_rule_id(language: LanguageFamily) -> RuleId {
    match language {
        LanguageFamily::Java => JAVA_REDUNDANT_ELSE_AFTER_RETURN,
        LanguageFamily::JsTs => JSTS_REDUNDANT_ELSE_AFTER_RETURN,
    }
}

/// D22: an IR node this rule scans for a terminator among its direct
/// statement children — the pre-IR `unreachable_container_kinds` grammar-
/// string list ("block"/"constructor_body" for Java, "statement_block"/
/// "program" for JS/TS), now a structural query: `node` is block-kind
/// exactly when one of its own children has `in_block` set (that flag
/// depends only on the child's parent's kind, so any one child settles it
/// for all of them; a `{ }` block's own braces are themselves `IrNode`
/// children, so this holds even for an empty block). JS/TS's top-level
/// `program` is the one case this structural test cannot see on its own —
/// `in_block` is deliberately false for it, since it is never itself a
/// braced block — so `is_root` (true only for `for_each_ir_node`'s first
/// call, on the file's own root) restores exactly that one exception, the
/// same way the pre-IR list did by naming it explicitly.
fn is_unreachable_container(node: &IrNode, is_root: bool, language: LanguageFamily) -> bool {
    (is_root && language == LanguageFamily::JsTs)
        || node.children.first().is_some_and(|child| child.in_block)
}

/// D22, `*-UNREACHABLE-AFTER-RETURN`: statements following an unconditional
/// `return`/`throw`/`break` that is itself a direct child of `node` (so a
/// terminator inside a nested `if` does not count — this rule never proves
/// reachability, only flags the syntactically obvious case). Flags every
/// statement after the first such terminator, to the end of the block,
/// except a JS/TS statement `is_hoisted_or_type_only` exempts — those are
/// not unreachable code in effect, only in source position.
fn find_unreachable_after_return(node: &IrNode) -> Option<(usize, usize, Vec<usize>)> {
    let statements = statement_children(node);
    let terminator_index = statements
        .iter()
        .position(|statement| ir::is_unreachable_terminator(statement))?;
    let unreachable: Vec<&IrNode> = statements[terminator_index + 1..]
        .iter()
        .copied()
        .filter(|statement| !statement.is_hoisted_or_type_only)
        .collect();
    let first = unreachable.first()?;
    let last = unreachable.last()?;
    let start_line = first.span.start_line as usize;
    let end_line = last.span.end_line as usize;
    let mut lines = BTreeSet::new();
    for statement in &unreachable {
        collect_ir_executable_lines(statement, &mut lines);
    }
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// D22, `*-EMPTY-CATCH`: a `catch` clause whose body block has neither a
/// statement nor a comment. `body`'s own IR children include both named
/// statements and a comment (also named in tree-sitter's own sense, and
/// thus already excluded here the same way `is_named` excludes it as a
/// `statement_children` member elsewhere) — counting any named child, not
/// just non-comment ones, is what makes the documented-empty (comment-only)
/// case decline.
fn find_empty_catch(node: &IrNode) -> Option<(usize, usize, Vec<usize>)> {
    let body = node.children.iter().find(|child| child.in_catch_body)?;
    if body.children.iter().any(|child| child.is_named) {
        return None;
    }
    let start_line = node.span.start_line as usize;
    let end_line = node.span.end_line as usize;
    let mut lines = BTreeSet::new();
    collect_ir_executable_lines(node, &mut lines);
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// D22, `*-REDUNDANT-ELSE-AFTER-RETURN`: an `else` branch whose sibling
/// `if` branch always returns, by the conservative, syntax-only test in
/// `always_returns` (no dataflow: an if/else chain that returns on every
/// path but does not end in a bare `return`/`throw` is not flagged).
/// `if_statement`'s three named, non-comment children are always
/// `[condition, consequence, alternative]` in that order in both grammars
/// (confirmed against `tree-sitter-java`/`tree-sitter-javascript`'s own
/// `node-types.json` field declarations), so positions 1 and 2 are the
/// two branches without needing a field name off the IR.
fn find_redundant_else(node: &IrNode) -> Option<(usize, usize, Vec<usize>)> {
    let children = statement_children(node);
    let consequence = *children.get(1)?;
    let alternative = *children.get(2)?;
    if !always_returns(consequence) {
        return None;
    }
    let start_line = alternative.span.start_line as usize;
    let end_line = alternative.span.end_line as usize;
    let mut lines = BTreeSet::new();
    collect_ir_executable_lines(alternative, &mut lines);
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// Conservative "always returns": the node itself is a bare `return`/
/// `throw` (`ir::is_return_or_throw`), or it is a block-kind node (see
/// `is_unreachable_container`'s doc comment for how block-kind-ness is
/// read off the IR without a grammar string) whose *last* direct statement
/// is one. No recursion into nested `if`/`else` exhaustiveness — that would
/// need dataflow, which D22 rules out.
fn always_returns(node: &IrNode) -> bool {
    if ir::is_return_or_throw(node) {
        return true;
    }
    if node.children.first().is_some_and(|child| child.in_block) {
        return statement_children(node)
            .last()
            .is_some_and(|last| ir::is_return_or_throw(last));
    }
    false
}

/// `node`'s direct named, non-comment children, in source order —
/// `IrNode::is_named` + `::is_comment` over `IrNode::children`, which (unlike
/// tree-sitter's own `named_children()`) holds anonymous children too.
fn statement_children(node: &IrNode) -> Vec<&IrNode> {
    node.children
        .iter()
        .filter(|child| child.is_named && !child.is_comment)
        .collect()
}

/// D11: every distinct 1-based source line within `node`'s own IR subtree
/// that has at least one node the lowering already marked `executable`
/// (`exec_lines::is_executable_leaf`, computed once per node during
/// lowering) — an iterative walk over `IrNode::children`'s already-built
/// `Vec`s, so it needs no cursor/parent bookkeeping the way a tree-sitter
/// walk does.
fn collect_ir_executable_lines(node: &IrNode, lines: &mut BTreeSet<usize>) {
    let mut stack: Vec<&IrNode> = vec![node];
    while let Some(current) = stack.pop() {
        if current.executable {
            let start = current.span.start_line as usize;
            let end = current.span.end_line as usize;
            for line in start..=end {
                lines.insert(line);
            }
        }
        stack.extend(current.children.iter());
    }
}

/// Visits `root` and every descendant, iteratively. `is_root` is `true`
/// only for the single call on `root` itself — see
/// `is_unreachable_container`'s doc comment for why that matters.
fn for_each_ir_node<'a>(root: &'a IrNode, mut visit: impl FnMut(&'a IrNode, bool)) {
    let mut stack: Vec<(&'a IrNode, bool)> = vec![(root, true)];
    while let Some((node, is_root)) = stack.pop() {
        visit(node, is_root);
        for child in node.children.iter().rev() {
            stack.push((child, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::FileScanSummary;

    fn parse_java_inline(source: &str) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("java grammar");
        let tree = parser.parse(source, None).expect("java parse");
        ParsedFile {
            relative_path: PathBuf::from("Inline.java"),
            language: LanguageFamily::Java,
            source: source.to_string(),
            tree,
        }
    }

    /// D12/D11: `file_language_lines`'s executable-line set must come from
    /// the IR's own `executable` spans (`collect_ir_executable_lines`), not
    /// a tree-sitter walk — the now-deleted `exec_lines::collect_executable_
    /// lines` was the pre-retarget production caller this replaces (see
    /// `:88:file_language_lines`'s doc comment). The expected set below is
    /// the same answer that function gave for this exact fixture before its
    /// removal: named, non-comment leaf tokens contribute their own line
    /// (`class`/`void`/`{`/`}` are anonymous and excluded; the comment on
    /// line 4 is excluded by kind), and the bare `return;` on line 5
    /// contributes via the bare-control-flow special case — but the two
    /// brace-only lines (6, 7) contribute nothing.
    #[test]
    fn test_file_language_lines_come_from_ir_spans() {
        let source = "class C {\n    void m() {\n        int x = 1;\n        // comment\n        return;\n    }\n}\n";
        let file = parse_java_inline(source);
        let metrics = MetricsResult {
            callables: Vec::new(),
            syntax_blocks: Vec::new(),
            file_scan_summaries: vec![FileScanSummary {
                relative_path: file.relative_path.clone(),
                scanned_lines: 7,
            }],
            erosion: 0.0,
            top25: Vec::new(),
            incomplete: false,
        };
        let files = file_language_lines(std::slice::from_ref(&file), &metrics);
        assert_eq!(files.len(), 1);
        let expected: BTreeSet<usize> = [1usize, 2, 3, 5].into_iter().collect();
        assert_eq!(files[0].executable_lines, expected, "{:?}", files[0]);
        assert_eq!(files[0].scanned_lines, 7);
        assert_eq!(files[0].language, LanguageFamily::Java);
    }
}

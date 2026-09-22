//! Stage 4 (WS-4): wasteful-code rules and the adapted verbosity score
//! (D22, D23). Six conservative, syntax-only rules — documented with their
//! conservative-by-design rationale in `docs/wasteful-rules.md` — run over
//! WS-2's parsed trees. This is an adaptation of scb-check's Python-only
//! wasteful-code rules, not a port and not a claim of numerical
//! equivalence — see `docs/wasteful-rules.md`.
//!
//! D11's per-line "named, non-comment leaf" rule (`is_comment_kind`,
//! `collect_executable_lines`, and the `is_executable_leaf` predicate
//! `collect_executable_lines` applies) is shared, imported from
//! `src/exec_lines.rs`; only `for_each_descendant`, this module's own
//! traversal helper, stays independently implemented here.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;
use tree_sitter::Node;

use crate::clones::redundant_occurrences;
use crate::exec_lines::{collect_executable_lines, is_comment_kind};
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

/// D22: the node kinds whose direct statement children the unreachable-
/// after-return rule scans — the same "block" concept in each language.
fn unreachable_container_kinds(language: LanguageFamily) -> &'static [&'static str] {
    match language {
        LanguageFamily::Java => &["block", "constructor_body"],
        LanguageFamily::JsTs => &["statement_block", "program"],
    }
}

/// D22: a statement kind that unconditionally exits its enclosing block.
const UNREACHABLE_TERMINATOR_KINDS: &[&str] =
    &["return_statement", "throw_statement", "break_statement"];

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
/// and D11-filtered executable-line set its own parsed file carries —
/// `FileScanSummary` alone has neither. The tree walk this does per file
/// (`collect_executable_lines`) is the only full-tree pass on this path, so
/// it fans out with rayon (D21) like every other whole-corpus pass in the
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
                    let mut executable_lines = BTreeSet::new();
                    collect_executable_lines(
                        file.tree.root_node(),
                        file.language,
                        &mut executable_lines,
                    );
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
/// detection time, see `collect_executable_lines`); a clone group
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
/// whole tree (no D9 nested-callable exclusion — a rule applies inside a
/// nested callable's body too).
fn scan_file_for_rules(file: &ParsedFile) -> Vec<RuleFinding> {
    let mut findings = Vec::new();
    let language = file.language;
    for_each_descendant(file.tree.root_node(), |node| {
        if unreachable_container_kinds(language).contains(&node.kind()) {
            if let Some((start_line, end_line, flagged_lines)) =
                find_unreachable_after_return(node, language)
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
        if node.kind() == "catch_clause" {
            if let Some((start_line, end_line, flagged_lines)) = find_empty_catch(node, language) {
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
        if node.kind() == "if_statement" {
            if let Some((start_line, end_line, flagged_lines)) = find_redundant_else(node, language)
            {
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

/// D22, `*-UNREACHABLE-AFTER-RETURN`: statements following an unconditional
/// `return`/`throw`/`break` that is itself a direct child of `node` (so a
/// terminator inside a nested `if` does not count — this rule never proves
/// reachability, only flags the syntactically obvious case). Flags every
/// statement after the first such terminator, to the end of the block,
/// except a JS/TS statement `is_hoisted_or_type_only` exempts — those are
/// not unreachable code in effect, only in source position.
fn find_unreachable_after_return(
    node: Node,
    language: LanguageFamily,
) -> Option<(usize, usize, Vec<usize>)> {
    let statements = statement_children(node, language);
    let terminator_index = statements
        .iter()
        .position(|statement| UNREACHABLE_TERMINATOR_KINDS.contains(&statement.kind()))?;
    let unreachable: Vec<Node> = statements[terminator_index + 1..]
        .iter()
        .copied()
        .filter(|statement| !is_hoisted_or_type_only(*statement, language))
        .collect();
    let first = unreachable.first()?;
    let last = unreachable.last()?;
    let start_line = first.start_position().row + 1;
    let end_line = last.end_position().row + 1;
    let mut lines = BTreeSet::new();
    for statement in &unreachable {
        collect_executable_lines(*statement, language, &mut lines);
    }
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// D22 exception, JS/TS only: a statement kind that is not actually
/// unreachable code in effect even when it sits after an unconditional
/// terminator. `function_declaration`/`generator_function_declaration` are
/// hoisted — they run regardless of where they sit in their block — and
/// `type_alias_declaration`/`interface_declaration` are type-only and
/// erased at runtime, so neither can be "dead code" in the sense this rule
/// means. The Java path has no such exception: Java has no hoisting and no
/// type-only declaration statement.
fn is_hoisted_or_type_only(node: Node, language: LanguageFamily) -> bool {
    language == LanguageFamily::JsTs
        && matches!(
            node.kind(),
            "function_declaration"
                | "generator_function_declaration"
                | "type_alias_declaration"
                | "interface_declaration"
        )
}

/// D22, `*-EMPTY-CATCH`: a `catch_clause` whose `body` block has neither a
/// statement nor a comment (a comment is a named child too, so it already
/// excludes the documented-empty case).
fn find_empty_catch(node: Node, language: LanguageFamily) -> Option<(usize, usize, Vec<usize>)> {
    let body = node.child_by_field_name("body")?;
    if body.named_child_count() != 0 {
        return None;
    }
    let start_line = node.start_position().row + 1;
    let end_line = node.end_position().row + 1;
    let mut lines = BTreeSet::new();
    collect_executable_lines(node, language, &mut lines);
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// D22, `*-REDUNDANT-ELSE-AFTER-RETURN`: an `else` branch whose sibling
/// `if` branch always returns, by the conservative, syntax-only test in
/// `always_returns` (no dataflow: an if/else chain that returns on every
/// path but does not end in a bare `return`/`throw` is not flagged).
fn find_redundant_else(node: Node, language: LanguageFamily) -> Option<(usize, usize, Vec<usize>)> {
    let consequence = node.child_by_field_name("consequence")?;
    let alternative = node.child_by_field_name("alternative")?;
    if !always_returns(consequence, language) {
        return None;
    }
    let start_line = alternative.start_position().row + 1;
    let end_line = alternative.end_position().row + 1;
    let mut lines = BTreeSet::new();
    collect_executable_lines(alternative, language, &mut lines);
    Some((start_line, end_line, lines.into_iter().collect()))
}

/// Conservative "always returns": the node itself is a bare `return`/
/// `throw`, or it is a block/`statement_block` whose *last* direct
/// statement is one. No recursion into nested `if`/`else` exhaustiveness —
/// that would need dataflow, which D22 rules out.
fn always_returns(node: Node, language: LanguageFamily) -> bool {
    match node.kind() {
        "return_statement" | "throw_statement" => true,
        "block" | "statement_block" => statement_children(node, language)
            .last()
            .map(|last| matches!(last.kind(), "return_statement" | "throw_statement"))
            .unwrap_or(false),
        _ => false,
    }
}

/// `node`'s direct named, non-comment children, in source order.
fn statement_children<'tree>(node: Node<'tree>, language: LanguageFamily) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| !is_comment_kind(child.kind(), language))
        .collect()
}

/// Iterative pre-order traversal via a single reused `TreeCursor` (no
/// native call-stack growth): visits `root` and every descendant.
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

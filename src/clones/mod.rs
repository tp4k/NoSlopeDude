//! Stage 3 (WS-3): duplicate-block detection. Enumerates candidate AST
//! statement blocks (D15), normalizes them into a 128-bit fingerprint of an
//! exact token stream that drops formatting and comments while preserving
//! identifiers and literals (D14), groups every matching location, drops a
//! group whose occurrences are all contained in a larger reported group's
//! (D15's maximal filter), and ranks the survivors by redundant lines beyond
//! the first occurrence (D14). `docs/clone-detection.md` documents the
//! algorithm; this module implements it.
//!
//! D11's per-line "named, non-comment leaf" rule that measures a block's
//! source lines now comes from the IR's own `executable` flag
//! (`accumulate_line` below); `accumulate_line` stays independently
//! implemented here — it counts a run's total lines with a
//! `last_counted_line` watermark, a different contract from
//! `rules::executable_lines_from_ir`'s distinct-line `Vec`
//! (`src/rules/mod.rs`). `is_comment_kind` (from
//! `src/exec_lines.rs`) and `tree_sitter::Node` are only reachable from the
//! `#[cfg(test)]` pre-IR reference implementation kept below for WS-1's
//! pinned floor test; the retargeted production path uses neither.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use rayon::prelude::*;
#[cfg(test)]
use tree_sitter::Node;

#[cfg(test)]
use crate::exec_lines::{for_each_descendant, is_comment_kind};
use crate::hashing::Digest;
use crate::ir::IrNode;
use crate::lower;
use crate::model::{CloneGroup, CloneLocation, ClonesResult, LanguageFamily};
use crate::parse::ParsedFile;

/// D15's minimum candidate size: fewer than two statements is never a
/// clone, however long the one statement is.
const MIN_CANDIDATE_STATEMENTS: usize = 2;

/// D14's separator between adjacent leaf tokens in a run's normalized
/// stream — a control character that cannot appear in Java/JS/TS source
/// text, so it can never be confused with token content.
const TOKEN_SEPARATOR: char = '\u{1}';

/// One candidate or grouped occurrence before it is exposed as a
/// `CloneLocation` — carries `statement_count` too, needed only internally
/// to order groups largest-first for the maximal-group filter. Carries the
/// owning file as a `file_index` into the file slice the enumeration was
/// run over, rather than a cloned `PathBuf`: this candidate shape is
/// produced `Θ(n²)` times per container, so an index avoids one heap
/// allocation per candidate (perf row 6); it is resolved to a real
/// `PathBuf` once, in `GroupBuilder::into_group`, only for the far smaller
/// set of candidates that end up in a reported group. `container` is the
/// ordinal of the run's container in `clone_containers`, and
/// `first_statement` the index of its first statement there.
#[derive(Clone, Copy)]
pub(crate) struct Candidate {
    pub(crate) file_index: u32,
    pub(crate) container: u32,
    pub(crate) first_statement: u32,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) source_lines: usize,
    pub(crate) statement_count: usize,
}

/// A clone group while it is still being assembled, before its
/// `redundant_lines` total is computed.
pub(crate) struct GroupBuilder {
    pub(crate) language: LanguageFamily,
    pub(crate) members: Vec<Candidate>,
    pub(crate) statement_count: usize,
}

impl GroupBuilder {
    pub(crate) fn from_candidates(language: LanguageFamily, candidates: Vec<Candidate>) -> Self {
        let statement_count = candidates
            .first()
            .map(|candidate| candidate.statement_count)
            .unwrap_or(0);
        GroupBuilder {
            language,
            members: candidates,
            statement_count,
        }
    }

    pub(crate) fn into_group(self, parsed_files: &[ParsedFile]) -> CloneGroup {
        self.into_group_with_paths(|file_index| {
            parsed_files[file_index as usize].relative_path.clone()
        })
    }

    /// `into_group` for a caller whose files are not `ParsedFile`s:
    /// `path_of` resolves a member's `file_index` to its path.
    pub(crate) fn into_group_with_paths(self, path_of: impl Fn(u32) -> PathBuf) -> CloneGroup {
        let mut locations: Vec<CloneLocation> = self
            .members
            .into_iter()
            .map(|candidate| CloneLocation {
                relative_path: path_of(candidate.file_index),
                start_line: candidate.start_line,
                end_line: candidate.end_line,
                source_lines: candidate.source_lines,
            })
            .collect();
        // D14: canonical order is `(path, start line)`; `locations[0]` is
        // then the group's "first occurrence".
        locations.sort_by(|a, b| {
            a.relative_path
                .cmp(&b.relative_path)
                .then(a.start_line.cmp(&b.start_line))
        });
        let redundant_lines: usize = locations[1..]
            .iter()
            .map(|location| location.source_lines)
            .sum();
        CloneGroup {
            language: self.language,
            locations,
            redundant_lines,
        }
    }
}

/// D14: the group's locations in canonical `(path, start line)` order with
/// the first (canonically earliest) location dropped. The single
/// definition of "beyond the first occurrence": this stream's own ranking
/// and WS-4's verbosity numerator (D23, Q1 answer b) both consume it rather
/// than each deciding "first" for themselves.
pub fn redundant_occurrences(group: &CloneGroup) -> &[CloneLocation] {
    group.locations.get(1..).unwrap_or(&[])
}

/// Runs the clones stage over every successfully parsed file: enumerates
/// candidates (D15), groups them by fingerprint (D14), drops subsumed
/// groups, and ranks the rest by redundant lines descending.
///
/// Lowers each file itself (`lower::lower_all`) and delegates to
/// `run_with_ir` below -- a thin wrapper kept so this signature's existing
/// call sites (mostly tests) stay untouched.
pub fn run(parsed_files: &[ParsedFile], min_clone_lines: u32) -> ClonesResult {
    let ir_files = lower::lower_all(parsed_files);
    run_with_ir(parsed_files, &ir_files, min_clone_lines)
}

/// WS-9 (C1): identical to `run` above, but takes the pipeline's own
/// single lowering pass instead of lowering `parsed_files` again --
/// `pipeline::run`'s production path calls this directly, `ir_files`
/// index-aligned with `parsed_files` (`lower::lower_all`'s own
/// `par_iter` preserves order). Enumerates candidates (D15) in parallel —
/// one rayon task per file, matching the upstream `parse`/`metrics` stages
/// (perf row 3; `enumerate_candidates` is pure per file and shares
/// nothing). Output order does not depend on this parallelism:
/// `GroupBuilder` sorts each group's locations canonically and the final
/// sort below is a total order over the resulting groups.
pub(crate) fn run_with_ir(
    parsed_files: &[ParsedFile],
    ir_files: &[lower::IrFile],
    min_clone_lines: u32,
) -> ClonesResult {
    debug_assert_eq!(parsed_files.len(), ir_files.len());
    let per_file: Vec<(LanguageFamily, Vec<(u128, Candidate)>)> = parsed_files
        .par_iter()
        .zip(ir_files.par_iter())
        .enumerate()
        .map(|(file_index, (file, ir_file))| {
            (
                file.language,
                enumerate_candidates(
                    file.language,
                    &file.source,
                    ir_file,
                    file_index as u32,
                    min_clone_lines,
                ),
            )
        })
        .collect();

    let mut groups: Vec<CloneGroup> = maximal_groups(per_file)
        .into_iter()
        .map(|builder| builder.into_group(parsed_files))
        .collect();

    groups.sort_by(|a, b| {
        b.redundant_lines.cmp(&a.redundant_lines).then_with(|| {
            let a_first = &a.locations[0];
            let b_first = &b.locations[0];
            a_first
                .relative_path
                .cmp(&b_first.relative_path)
                .then(a_first.start_line.cmp(&b_first.start_line))
        })
    });

    ClonesResult { groups }
}

/// D14's grouping and D15's maximal filter over every file's candidates:
/// each fingerprint with at least two candidates is a group, and a group
/// all of whose occurrences lie inside a larger group's is dropped. The
/// groups come back in no particular order and with their members in no
/// particular order; callers sort. `file_index` in each member indexes the
/// slice the per-file enumerations ran over.
pub(crate) fn maximal_groups(
    per_file: Vec<(LanguageFamily, Vec<(u128, Candidate)>)>,
) -> Vec<GroupBuilder> {
    let mut candidates_by_key: HashMap<u128, (LanguageFamily, Vec<Candidate>)> = HashMap::new();
    for (language, candidates) in per_file {
        for (key, candidate) in candidates {
            candidates_by_key
                .entry(key)
                .or_insert_with(|| (language, Vec::new()))
                .1
                .push(candidate);
        }
    }

    let builders: Vec<GroupBuilder> = candidates_by_key
        .into_values()
        .filter(|(_, candidates)| candidates.len() >= 2)
        .map(|(language, candidates)| GroupBuilder::from_candidates(language, candidates))
        .collect();

    drop_subsumed_groups(builders)
}

/// D15's maximal filter: a group all of whose occurrences are contained in
/// another (larger) reported group's occurrences is dropped, so one long
/// duplicate does not also report all its sub-runs. Processes larger
/// groups (by statement count) first, so a kept group is never itself
/// later found to be subsumed by a smaller one.
fn drop_subsumed_groups(mut builders: Vec<GroupBuilder>) -> Vec<GroupBuilder> {
    builders.sort_by_key(|builder| std::cmp::Reverse(builder.statement_count));
    let mut kept: Vec<GroupBuilder> = Vec::with_capacity(builders.len());
    // File index -> every kept group's per-member line range in that file,
    // tagged with its index into `kept`. A candidate is tested only
    // against the kept groups whose range covers one of its own members,
    // never against the whole `kept` vector (perf row 4).
    let mut kept_by_file: HashMap<u32, Vec<(usize, usize, usize)>> = HashMap::new();
    for builder in builders {
        let subsumed = is_subsumed_by_any_kept(&builder, &kept, &kept_by_file);
        if !subsumed {
            let kept_index = kept.len();
            for member in &builder.members {
                kept_by_file.entry(member.file_index).or_default().push((
                    member.start_line,
                    member.end_line,
                    kept_index,
                ));
            }
            kept.push(builder);
        }
    }
    kept
}

/// Whether some already-kept, still-larger group contains every member of
/// `builder` (D15's maximal filter). Containing any one member is necessary
/// for any `other` to subsume `builder` at all, so the kept groups to run
/// the full `is_subsumed` check against are narrowed to the ones with a
/// member covering the first one, via `kept_by_file`, before that check
/// runs (perf row 4).
fn is_subsumed_by_any_kept(
    builder: &GroupBuilder,
    kept: &[GroupBuilder],
    kept_by_file: &HashMap<u32, Vec<(usize, usize, usize)>>,
) -> bool {
    let first = &builder.members[0];
    let Some(entries) = kept_by_file.get(&first.file_index) else {
        return false;
    };
    let mut candidate_indices: Vec<usize> = entries
        .iter()
        .filter(|(start_line, end_line, _)| {
            *start_line <= first.start_line && first.end_line <= *end_line
        })
        .map(|(_, _, kept_index)| *kept_index)
        .collect();
    candidate_indices.sort_unstable();
    candidate_indices.dedup();
    candidate_indices.into_iter().any(|kept_index| {
        let other = &kept[kept_index];
        // Equal or smaller statement count cannot strictly contain
        // `builder`; `kept` is otherwise already sorted largest-first.
        other.statement_count > builder.statement_count && is_subsumed(builder, other)
    })
}

/// Whether every member in `candidate` sits inside some member of `other`,
/// in the same file.
fn is_subsumed(candidate: &GroupBuilder, other: &GroupBuilder) -> bool {
    candidate.members.iter().all(|member| {
        other.members.iter().any(|other_member| {
            other_member.file_index == member.file_index
                && other_member.start_line <= member.start_line
                && member.end_line <= other_member.end_line
        })
    })
}

/// D15: every candidate block in `file` that passes the ≥2-statement and
/// ≥`min_clone_lines` filters, as `(fingerprint, candidate)` pairs. Walks
/// `lower::lower_file`'s `IrNode` tree rather than `file.tree`'s
/// tree-sitter nodes: every `IrNode` whose direct children include at
/// least one `is_clone_statement` node is a clone-candidate container
/// (D15's six kinds, all now expressed as that one IR flag — see
/// `ir::IrNode::is_clone_statement`'s own doc comment), so no
/// `(language, "<grammar kind>")` match is needed to find them. Most
/// nodes contribute no `is_clone_statement` children at all, and
/// `enumerate_container_candidates` itself returns immediately on a list
/// shorter than `MIN_CANDIDATE_STATEMENTS`, so calling it unconditionally
/// costs one cheap length check per node rather than a container-kind
/// dispatch.
/// WS-9 (C1): `ir_file` is the caller's own lowering (`run_with_ir`'s
/// `ir_files`, index-aligned with `parsed_files`), not lowered again here.
pub(crate) fn enumerate_candidates(
    language: LanguageFamily,
    source: &str,
    ir_file: &lower::IrFile,
    file_index: u32,
    min_clone_lines: u32,
) -> Vec<(u128, Candidate)> {
    let mut candidates = Vec::new();
    for (container, statements) in clone_containers(ir_file).iter().enumerate() {
        enumerate_container_candidates(
            statements,
            language,
            (file_index, container as u32),
            min_clone_lines,
            source,
            &mut candidates,
        );
    }
    candidates
}

/// Every clone-candidate container of `ir_file` that holds at least
/// `MIN_CANDIDATE_STATEMENTS` clone statements, as its statement list, in
/// pre-order. A `Candidate::container` is an index into this list.
pub(crate) fn clone_containers(ir_file: &lower::IrFile) -> Vec<Vec<&IrNode>> {
    let mut containers = Vec::new();
    for_each_ir_node(&ir_file.root, &mut |node| {
        let statements: Vec<&IrNode> = node
            .children
            .iter()
            .filter(|child| child.is_clone_statement)
            .collect();
        if statements.len() >= MIN_CANDIDATE_STATEMENTS {
            containers.push(statements);
        }
    });
    containers
}

/// D15's ≥2-statement, ≥`min_clone_lines` filters over every contiguous
/// sibling run in one container, computed incrementally (perf row 1): for
/// a fixed `start`, each `end` extends the previous D11 line count and D14
/// fingerprint by exactly one statement's leaves, rather than re-walking
/// `[start..end]` from scratch — the running state after `[start..end]` is
/// exactly the state `[start..end+1]` needs, the same equivalence WS-2's
/// `accumulate_line` already relies on. This takes one container from
/// `Θ(n³·tokens)` to `Θ(n·tokens + n²)`: the per-statement token cache
/// below is built once regardless of `start`, and the per-`start` loop
/// re-walks each statement's leaves at most once per `start` value for the
/// line count, which cannot itself be cached the same way (its running
/// `last_counted_line` state depends on the previous statement in the run).
fn enumerate_container_candidates(
    statements: &[&IrNode],
    language: LanguageFamily,
    (file_index, container): (u32, u32),
    min_clone_lines: u32,
    source: &str,
    out: &mut Vec<(u128, Candidate)>,
) {
    let statement_count = statements.len();
    if statement_count < MIN_CANDIDATE_STATEMENTS {
        return;
    }
    let statement_tokens: Vec<String> = statements
        .iter()
        .map(|&statement| ir_statement_tokens(statement, source))
        .collect();
    let prefix = family_prefix(language);

    for start in 0..statement_count {
        let mut fingerprint = RunFingerprint::new(prefix);
        let mut source_lines = 0usize;
        let mut last_counted_line = 0usize;
        for end in (start + 1)..=statement_count {
            let statement = statements[end - 1];
            fingerprint.push(&statement_tokens[end - 1]);
            for_each_ir_node(statement, &mut |node| {
                if node.executable {
                    accumulate_line(node, &mut source_lines, &mut last_counted_line);
                }
            });

            let run_statement_count = end - start;
            if run_statement_count < MIN_CANDIDATE_STATEMENTS
                || source_lines < min_clone_lines as usize
            {
                continue;
            }

            let start_line = statements[start].span.start_line as usize;
            let end_line = statement.span.end_line as usize;
            out.push((
                fingerprint.finish(),
                Candidate {
                    file_index,
                    container,
                    first_statement: start as u32,
                    start_line,
                    end_line,
                    source_lines,
                    statement_count: run_statement_count,
                },
            ));
        }
    }
}

/// D14: one statement's own contribution to the normalized token stream,
/// derived from its `IrNode` subtree rather than a tree-sitter walk: every
/// leaf descendant (`node.children.is_empty()`), in order, with a comment
/// leaf (`ir::IrNode::is_comment`) dropped. A named leaf's own source text
/// (`node.span`) is preserved verbatim. `ir::IrNode::is_named` still needs
/// its own branch for the remaining (anonymous) leaves: an anonymous
/// grammar token's own source text is usually its exact `kind()` string,
/// but not always — `tree-sitter-javascript` 0.25.0 aliases
/// `seq('static', /\s+/, 'get', /\s*\n/)` to the single anonymous kind
/// `"static get"` (`grammar.js:1252`), whose own source text carries
/// whatever internal whitespace and trailing newline the author wrote.
/// Whitespace-collapsing an anonymous leaf's own text reproduces its
/// `kind()` exactly for that token while staying the identity transform
/// for the (overwhelmingly more common) whitespace-free anonymous tokens,
/// so two copy-pasted blocks that differ only in that token's internal
/// formatting still fingerprint identically. Cached once per container,
/// per perf row 1, so a multi-statement run's fingerprint is built by
/// feeding these in sequence, never by re-walking the statements it
/// already covers.
pub(crate) fn ir_statement_tokens(statement: &IrNode, source: &str) -> String {
    let mut tokens = String::new();
    for token in statement_leaf_tokens(statement, source) {
        tokens.push(TOKEN_SEPARATOR);
        tokens.push_str(&token);
    }
    tokens
}

/// D14/D10: one statement's normalized leaf tokens as a sequence, the same
/// tokens `ir_statement_tokens` joins, so containment can compare whole
/// tokens instead of searching the joined string.
pub(crate) fn statement_leaf_tokens<'a>(statement: &IrNode, source: &'a str) -> Vec<Cow<'a, str>> {
    let mut tokens = Vec::new();
    for_each_ir_node(statement, &mut |node| {
        if !node.children.is_empty() || node.is_comment {
            return;
        }
        let text = leaf_text(node, source);
        if node.is_named || !text.chars().any(char::is_whitespace) {
            tokens.push(Cow::Borrowed(text));
        } else {
            tokens.push(Cow::Owned(
                text.split_whitespace().collect::<Vec<_>>().join(" "),
            ));
        }
    });
    tokens
}

/// D11: the source lines a run of sibling statements counts, ascending and
/// distinct, exactly the lines `accumulate_line` adds up for `source_lines`.
pub(crate) fn run_executable_lines(statements: &[&IrNode]) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut last_counted_line = 0usize;
    for statement in statements {
        for_each_ir_node(statement, &mut |node| {
            if node.executable {
                if let Some(fresh) = uncounted_lines(node, last_counted_line) {
                    last_counted_line = *fresh.end();
                    lines.extend(fresh);
                }
            }
        });
    }
    lines
}

/// The source text at one `IrNode`'s own span — a byte slice, not a
/// tree-sitter `utf8_text` call, since `IrNode` carries no reference back
/// to a tree-sitter node.
pub(crate) fn leaf_text<'a>(node: &IrNode, source: &'a str) -> &'a str {
    source
        .get(node.span.start_byte as usize..node.span.end_byte as usize)
        .unwrap_or("")
}

/// The pre-IR reference implementation, kept verbatim: `nsd-plan-final.md`
/// M0b item 4's floor test (`src/lower/mod.rs::tests::
/// test_ir_tokens_reproduce_the_pre_ir_stream_on_every_fixture`, WS-1's,
/// out of this stream's file scope) calls this exact function, by this
/// exact name, with a tree-sitter `Node` argument -- so its signature
/// cannot change without breaking that pinned, unmodifiable call site (see
/// this round's Open questions). The retargeted production path above
/// (`ir_statement_tokens`) is what `enumerate_container_candidates` now
/// calls; this function is unreachable from `clones::run` after the
/// retarget. `#[cfg(test)]`, not `#[allow(dead_code)]`: its one caller is
/// itself a `#[cfg(test)]` item (WS-1's), so outside a test build it is not
/// suppressed dead code, it genuinely does not exist -- and both vanish
/// together, so the same `#[cfg(test)]` build that omits this omits the
/// caller too.
#[cfg(test)]
pub(crate) fn normalized_statement_tokens(
    statement: Node,
    language: LanguageFamily,
    source: &str,
) -> String {
    let mut tokens = String::new();
    for_each_descendant(statement, |node| {
        if node.child_count() != 0 || is_comment_kind(node.kind(), language) {
            return;
        }
        tokens.push(TOKEN_SEPARATOR);
        if node.is_named() {
            tokens.push_str(node.utf8_text(source.as_bytes()).unwrap_or(""));
        } else {
            tokens.push_str(node.kind());
        }
    });
    tokens
}

/// D14's grouping key: a versioned BLAKE3 digest (`src/hashing.rs`) of a
/// run's normalized token stream (family prefix, then every statement's
/// `normalized_statement_tokens` in order), never materialised as the
/// concatenated `String` itself (perf row 2 — coordinator-approved change
/// from D14's original "exact normalized token strings, not hashes": the
/// collision probability at these candidate volumes is a non-concern in
/// practice, the same principle content-addressed systems rely on). WS-3
/// replaces the two SipHash streams (`std`'s prior default hasher) this used
/// to be with one digest safe to persist once M5's cache keys on it.
struct RunFingerprint {
    digest: Digest,
}

impl RunFingerprint {
    fn new(family_prefix: &str) -> Self {
        RunFingerprint {
            digest: Digest::new(family_prefix),
        }
    }

    fn push(&mut self, statement_tokens: &str) {
        self.digest.push(statement_tokens.as_bytes());
    }

    fn finish(&self) -> u128 {
        self.digest.finish()
    }
}

fn family_prefix(language: LanguageFamily) -> &'static str {
    match language {
        LanguageFamily::Java => "java",
        LanguageFamily::JsTs => "js_ts",
    }
}

/// Adds a leaf's not-yet-counted lines to `count`, relying on
/// `for_each_ir_node` visiting leaves in non-decreasing source-line order,
/// the same technique WS-2's callable SLOC uses.
fn accumulate_line(node: &IrNode, count: &mut usize, last_counted_line: &mut usize) {
    if let Some(fresh) = uncounted_lines(node, *last_counted_line) {
        *count += fresh.end() - fresh.start() + 1;
        *last_counted_line = *fresh.end();
    }
}

/// The lines of a leaf not yet counted past `last_counted_line`, if any.
fn uncounted_lines(node: &IrNode, last_counted_line: usize) -> Option<RangeInclusive<usize>> {
    let start = node.span.start_line as usize;
    let end = node.span.end_line as usize;
    let from = start.max(last_counted_line + 1);
    (from <= end).then_some(from..=end)
}

/// Iterative pre-order traversal of one already-built `IrNode` tree: visits
/// `root`, then each child in document order (`build_ir` assembles
/// `IrNode::children` in the same order its own `TreeCursor` walked them).
/// A stack of `&IrNode` refs, not recursion (D18: no per-AST-depth
/// recursion) -- the same reason `ir::IrNode`'s own `Drop` impl is
/// iterative.
fn for_each_ir_node<'a>(root: &'a IrNode, visit: &mut impl FnMut(&'a IrNode)) {
    let mut stack: Vec<&'a IrNode> = vec![root];
    while let Some(node) = stack.pop() {
        visit(node);
        stack.extend(node.children.iter().rev());
    }
}

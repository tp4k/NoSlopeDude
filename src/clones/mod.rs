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
//! source lines is shared: `is_executable_leaf` and `is_comment_kind` are
//! imported from `src/exec_lines.rs`. `accumulate_line` stays
//! independently implemented here — it counts a run's total lines with a
//! `last_counted_line` watermark, a different contract from
//! `exec_lines`'s distinct-line `BTreeSet`.

use std::collections::HashMap;
use std::path::PathBuf;

use rayon::prelude::*;
use tree_sitter::Node;

use crate::exec_lines::{is_comment_kind, is_executable_leaf};
use crate::hashing::Digest;
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
/// owning file as a `file_index` into the `ParsedFile` slice `run` was
/// called with, rather than a cloned `PathBuf`: this candidate shape is
/// produced `Θ(n²)` times per container, so an index avoids one heap
/// allocation per candidate (perf row 6); it is resolved to a real
/// `PathBuf` once, in `GroupBuilder::from_candidates`, only for the far
/// smaller set of candidates that end up in a reported group.
struct Candidate {
    file_index: u32,
    start_line: usize,
    end_line: usize,
    source_lines: usize,
    statement_count: usize,
}

/// A clone group while it is still being assembled, before its
/// `redundant_lines` total is computed.
struct GroupBuilder {
    language: LanguageFamily,
    locations: Vec<CloneLocation>,
    statement_count: usize,
}

impl GroupBuilder {
    fn from_candidates(
        language: LanguageFamily,
        candidates: Vec<Candidate>,
        parsed_files: &[ParsedFile],
    ) -> Self {
        let statement_count = candidates
            .first()
            .map(|candidate| candidate.statement_count)
            .unwrap_or(0);
        let mut locations: Vec<CloneLocation> = candidates
            .into_iter()
            .map(|candidate| CloneLocation {
                relative_path: parsed_files[candidate.file_index as usize]
                    .relative_path
                    .clone(),
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
        GroupBuilder {
            language,
            locations,
            statement_count,
        }
    }

    fn into_group(self) -> CloneGroup {
        let redundant_lines: usize = self.locations[1..]
            .iter()
            .map(|location| location.source_lines)
            .sum();
        CloneGroup {
            language: self.language,
            locations: self.locations,
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
/// candidates (D15) in parallel — one rayon task per file, matching the
/// upstream `parse`/`metrics` stages (perf row 3; `enumerate_candidates` is
/// pure per file and shares nothing) — groups them by fingerprint (D14),
/// drops subsumed groups, and ranks the rest by redundant lines descending.
/// Output order does not depend on this parallelism: `GroupBuilder` sorts
/// each group's locations canonically and the final sort below is a total
/// order over the resulting groups.
pub fn run(parsed_files: &[ParsedFile], min_clone_lines: u32) -> ClonesResult {
    let per_file: Vec<(LanguageFamily, Vec<(u128, Candidate)>)> = parsed_files
        .par_iter()
        .enumerate()
        .map(|(file_index, file)| {
            (
                file.language,
                enumerate_candidates(file, file_index as u32, min_clone_lines),
            )
        })
        .collect();

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
        .map(|(language, candidates)| {
            GroupBuilder::from_candidates(language, candidates, parsed_files)
        })
        .collect();

    let mut groups: Vec<CloneGroup> = drop_subsumed_groups(builders)
        .into_iter()
        .map(GroupBuilder::into_group)
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

/// D15's maximal filter: a group all of whose occurrences are contained in
/// another (larger) reported group's occurrences is dropped, so one long
/// duplicate does not also report all its sub-runs. Processes larger
/// groups (by statement count) first, so a kept group is never itself
/// later found to be subsumed by a smaller one.
fn drop_subsumed_groups(mut builders: Vec<GroupBuilder>) -> Vec<GroupBuilder> {
    builders.sort_by_key(|builder| std::cmp::Reverse(builder.statement_count));
    let mut kept: Vec<GroupBuilder> = Vec::with_capacity(builders.len());
    // Path -> every kept group's per-location line range in that file,
    // tagged with its index into `kept`. A candidate is tested only
    // against the kept groups whose range covers its own canonical first
    // location, never against the whole `kept` vector (perf row 4).
    let mut kept_by_path: HashMap<PathBuf, Vec<(usize, usize, usize)>> = HashMap::new();
    for builder in builders {
        let subsumed = is_subsumed_by_any_kept(&builder, &kept, &kept_by_path);
        if !subsumed {
            let kept_index = kept.len();
            for location in &builder.locations {
                kept_by_path
                    .entry(location.relative_path.clone())
                    .or_default()
                    .push((location.start_line, location.end_line, kept_index));
            }
            kept.push(builder);
        }
    }
    kept
}

/// Whether some already-kept, still-larger group contains every location of
/// `builder` (D15's maximal filter). Containing `builder.locations[0]` is
/// necessary for any `other` to subsume `builder` at all, so the kept
/// groups to run the full `is_subsumed` check against are narrowed to the
/// ones with a location covering it, via `kept_by_path`, before that check
/// runs (perf row 4).
fn is_subsumed_by_any_kept(
    builder: &GroupBuilder,
    kept: &[GroupBuilder],
    kept_by_path: &HashMap<PathBuf, Vec<(usize, usize, usize)>>,
) -> bool {
    let first = &builder.locations[0];
    let Some(entries) = kept_by_path.get(&first.relative_path) else {
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

/// Whether every location in `candidate` sits inside some location of
/// `other`, in the same file.
fn is_subsumed(candidate: &GroupBuilder, other: &GroupBuilder) -> bool {
    candidate.locations.iter().all(|location| {
        other.locations.iter().any(|other_location| {
            other_location.relative_path == location.relative_path
                && other_location.start_line <= location.start_line
                && location.end_line <= other_location.end_line
        })
    })
}

/// D15: every candidate block in `file` that passes the ≥2-statement and
/// ≥`min_clone_lines` filters, as `(fingerprint, candidate)` pairs.
fn enumerate_candidates(
    file: &ParsedFile,
    file_index: u32,
    min_clone_lines: u32,
) -> Vec<(u128, Candidate)> {
    let mut candidates = Vec::new();
    for_each_descendant(file.tree.root_node(), |node| {
        let Some(statements) = statement_children(node, file.language) else {
            return;
        };
        enumerate_container_candidates(
            &statements,
            file.language,
            file_index,
            min_clone_lines,
            &file.source,
            &mut candidates,
        );
    });
    candidates
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
    statements: &[Node],
    language: LanguageFamily,
    file_index: u32,
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
        .map(|&statement| normalized_statement_tokens(statement, language, source))
        .collect();
    let prefix = family_prefix(language);

    for start in 0..statement_count {
        let mut fingerprint = RunFingerprint::new(prefix);
        let mut source_lines = 0usize;
        let mut last_counted_line = 0usize;
        for end in (start + 1)..=statement_count {
            let statement = statements[end - 1];
            fingerprint.push(&statement_tokens[end - 1]);
            for_each_descendant(statement, |node| {
                if is_executable_leaf(node, language) {
                    accumulate_line(node, &mut source_lines, &mut last_counted_line);
                }
            });

            let run_statement_count = end - start;
            if run_statement_count < MIN_CANDIDATE_STATEMENTS
                || source_lines < min_clone_lines as usize
            {
                continue;
            }

            let start_line = statements[start].start_position().row + 1;
            let end_line = statement.end_position().row + 1;
            out.push((
                fingerprint.finish(),
                Candidate {
                    file_index,
                    start_line,
                    end_line,
                    source_lines,
                    statement_count: run_statement_count,
                },
            ));
        }
    }
}

/// D15's candidate containers: Java `block`, `constructor_body` and
/// `switch_block_statement_group`; JS/TS `statement_block`, the top-level
/// `program`, and the `body` field of a `switch_case`/`switch_default`.
/// Returns the container's direct statement children (comments and, for
/// the Java switch group, its `switch_label`s dropped), or `None` for
/// every other node kind. Not `(Java, "program")`: Java's `program` holds
/// type declarations, not statements.
fn statement_children<'tree>(
    node: Node<'tree>,
    language: LanguageFamily,
) -> Option<Vec<Node<'tree>>> {
    match (language, node.kind()) {
        (LanguageFamily::Java, "block") => Some(named_non_comment_children(node, language)),
        (LanguageFamily::Java, "constructor_body") => {
            Some(named_non_comment_children(node, language))
        }
        (LanguageFamily::Java, "switch_block_statement_group") => Some(
            named_non_comment_children(node, language)
                .into_iter()
                .filter(|child| child.kind() != "switch_label")
                .collect(),
        ),
        (LanguageFamily::JsTs, "statement_block") => {
            Some(named_non_comment_children(node, language))
        }
        (LanguageFamily::JsTs, "program") => Some(named_non_comment_children(node, language)),
        (LanguageFamily::JsTs, "switch_case" | "switch_default") => {
            let mut cursor = node.walk();
            Some(
                node.children_by_field_name("body", &mut cursor)
                    .filter(|child| !is_comment_kind(child.kind(), language))
                    .collect(),
            )
        }
        _ => None,
    }
}

fn named_non_comment_children<'tree>(
    node: Node<'tree>,
    language: LanguageFamily,
) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| !is_comment_kind(child.kind(), language))
        .collect()
}

/// D14: one statement's own contribution to the normalized token stream —
/// every leaf descendant, in order, with comments dropped, an anonymous
/// leaf reduced to its `node.kind()`, and a named leaf's (identifier or
/// literal) text preserved verbatim. Cached once per container, per perf
/// row 1, so a multi-statement run's fingerprint is built by feeding these
/// in sequence, never by re-walking the statements it already covers.
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
/// `for_each_descendant` visiting leaves in non-decreasing source-line
/// order, the same technique WS-2's callable SLOC uses.
fn accumulate_line(node: Node, count: &mut usize, last_counted_line: &mut usize) {
    let start = node.start_position().row + 1;
    let end = node.end_position().row + 1;
    let from = start.max(*last_counted_line + 1);
    if from <= end {
        *count += end - from + 1;
        *last_counted_line = end;
    }
}

/// Iterative pre-order traversal via a single reused `TreeCursor`: visits
/// `root` and every descendant. Unlike WS-2's `walk_excluding`, this stream
/// never needs to skip a subtree (no nested-callable exclusion applies to
/// measuring one clone span), so it carries no exclusion predicate.
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

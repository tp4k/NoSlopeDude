//! M4-2: V102, the clone-regression policy (`nsd-plan-final.md`
//! *Diagnostics*, A3).
//!
//! An occurrence is a D15 candidate run. A changed candidate occurrence with
//! an added executable line, inside a maximal candidate-side clone group, is
//! an extension of the base occurrence the diff maps it to, or else an added
//! occurrence that either pairs with a deleted base occurrence as a move or
//! is new. `docs/clone-detection.md` documents the rules.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use rayon::prelude::*;

use crate::clones::{
    clone_containers, enumerate_candidates, maximal_groups, run_executable_lines,
    statement_leaf_tokens, Candidate,
};
use crate::config::{PolicyConfig, Severity};
use crate::git::diff::{map_lines, Change, LineMap};
use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::ir::IrNode;
use crate::lower::IrFile;
use crate::model::LanguageFamily;
use crate::policy::diagnostics::{CloneDiagnostic, CODE_CLONE_REGRESSION};
use crate::policy::findings::FindingFile;

/// A3: an extension or move is within `max(1, floor(lines / 10))` extra
/// executable lines.
const THRESHOLD_DIVISOR: usize = 10;
const MIN_THRESHOLD: usize = 1;

/// Multiplier of the polynomial rolling hash over token hashes (the 64-bit
/// FNV prime, odd).
const TOKEN_HASH_BASE: u64 = 0x0000_0100_0000_01b3;

fn threshold(base_lines: usize) -> usize {
    (base_lines / THRESHOLD_DIVISOR).max(MIN_THRESHOLD)
}

type PerFileCandidates = Vec<(u128, Candidate)>;

/// Overlap, then size, then source order (earlier is greater).
type OverlapRank = (usize, usize, Reverse<usize>, Reverse<usize>);

/// One analyzed file with its clone containers, over the caller's analysis.
struct FileView<'a> {
    path: &'a RepoPath,
    source: &'a str,
    language: LanguageFamily,
    ir: &'a IrFile,
    containers: Vec<Vec<&'a IrNode>>,
    spans: Vec<(usize, usize)>,
}

impl<'a> FileView<'a> {
    /// `None` for a file whose bytes are not UTF-8: it has no analysis to
    /// enumerate (A102 is raised elsewhere).
    fn new(file: &'a FindingFile<'a>) -> Option<Self> {
        let source = std::str::from_utf8(file.source).ok()?;
        let ir = &file.analysis.ir;
        let containers = clone_containers(ir);
        let spans = containers
            .iter()
            .map(|statements| {
                let first = statements.first().map_or(0, |s| s.span.start_line as usize);
                let last = statements.last().map_or(0, |s| s.span.end_line as usize);
                (first, last)
            })
            .collect();
        Some(FileView {
            path: &file.path,
            source,
            language: file.analysis.language,
            ir,
            containers,
            spans,
        })
    }

    fn enumerate(&self, file_index: usize, min_clone_lines: u32) -> PerFileCandidates {
        enumerate_candidates(
            self.language,
            self.source,
            self.ir,
            file_index as u32,
            min_clone_lines,
        )
    }

    fn statements(&self, run: &Candidate) -> &[&'a IrNode] {
        let first = run.first_statement as usize;
        &self.containers[run.container as usize][first..first + run.statement_count]
    }

    fn executable_lines(&self, run: &Candidate) -> Vec<usize> {
        run_executable_lines(self.statements(run))
    }
}

fn token_hash(token: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

/// One container's whole-token sequence with a rolling-hash prefix table,
/// so the hash of any contiguous token window is O(1). Containment is
/// verified by comparing whole tokens (D10), never the joined digest string.
struct ContainerTokens<'a> {
    tokens: Vec<Cow<'a, str>>,
    bounds: Vec<usize>,
    prefix: Vec<u64>,
}

impl<'a> ContainerTokens<'a> {
    fn new(view: &FileView<'a>, container: u32) -> Self {
        let mut tokens = Vec::new();
        let mut bounds = vec![0];
        for statement in &view.containers[container as usize] {
            tokens.extend(statement_leaf_tokens(statement, view.source));
            bounds.push(tokens.len());
        }
        let mut prefix = Vec::with_capacity(tokens.len() + 1);
        prefix.push(0u64);
        let mut running = 0u64;
        for token in &tokens {
            running = running
                .wrapping_mul(TOKEN_HASH_BASE)
                .wrapping_add(token_hash(token));
            prefix.push(running);
        }
        ContainerTokens {
            tokens,
            bounds,
            prefix,
        }
    }

    fn run_range(&self, run: &Candidate) -> (usize, usize) {
        let first = run.first_statement as usize;
        (self.bounds[first], self.bounds[first + run.statement_count])
    }

    fn window_hash(&self, start: usize, end: usize) -> u64 {
        let scale = TOKEN_HASH_BASE.wrapping_pow((end - start) as u32);
        self.prefix[end].wrapping_sub(self.prefix[start].wrapping_mul(scale))
    }

    fn window(&self, start: usize, end: usize) -> &[Cow<'a, str>] {
        &self.tokens[start..end]
    }
}

/// How a changed base file's lines relate to its candidate.
enum BaseState {
    Deleted,
    Mapped(usize),
}

/// The diff-derived facts about changed files: line maps, the base
/// counterpart of each changed candidate file, and the base files whose
/// lines may be deleted.
struct ChangeFacts {
    maps: Vec<LineMap>,
    candidate_to_base: Vec<BTreeMap<usize, usize>>,
    counterparts: HashMap<usize, (usize, usize)>,
    base_states: BTreeMap<usize, BaseState>,
}

impl ChangeFacts {
    fn new(
        changes: &[Change],
        base_views: &[FileView<'_>],
        changed_views: &[FileView<'_>],
    ) -> Result<Self, GitError> {
        let base_index: HashMap<&RepoPath, usize> = base_views
            .iter()
            .enumerate()
            .map(|(index, view)| (view.path, index))
            .collect();
        let candidate_index: HashMap<&RepoPath, usize> = changed_views
            .iter()
            .enumerate()
            .map(|(index, view)| (view.path, index))
            .collect();
        let mut facts = ChangeFacts {
            maps: Vec::new(),
            candidate_to_base: Vec::new(),
            counterparts: HashMap::new(),
            base_states: BTreeMap::new(),
        };
        for change in changes {
            let (base_path, candidate_path) = match change {
                Change::Added { path, .. } => (None, Some(path)),
                Change::Modified { path, .. } | Change::Typechange { path, .. } => {
                    (Some(path), Some(path))
                }
                Change::Renamed { from, to, .. } => (Some(from), Some(to)),
                Change::Deleted { path, .. } => (Some(path), None),
            };
            let base = base_path.and_then(|path| base_index.get(path).copied());
            let candidate = candidate_path.and_then(|path| candidate_index.get(path).copied());
            match (base, candidate) {
                (Some(base), Some(candidate)) => {
                    let map = map_lines(
                        base_views[base].source.as_bytes(),
                        changed_views[candidate].source.as_bytes(),
                    )?;
                    let inverse = map
                        .base_to_candidate
                        .iter()
                        .map(|(&base_line, &candidate_line)| (candidate_line, base_line))
                        .collect();
                    let slot = facts.maps.len();
                    facts.maps.push(map);
                    facts.candidate_to_base.push(inverse);
                    facts.counterparts.insert(candidate, (base, slot));
                    facts.base_states.insert(base, BaseState::Mapped(slot));
                }
                (Some(base), None) if matches!(change, Change::Deleted { .. }) => {
                    facts.base_states.insert(base, BaseState::Deleted);
                }
                _ => {}
            }
        }
        Ok(facts)
    }

    fn is_deleted(&self, state: &BaseState, line: usize) -> bool {
        match state {
            BaseState::Deleted => true,
            BaseState::Mapped(slot) => self.maps[*slot].deleted_base_lines.contains(&line),
        }
    }

    fn added_executable_lines(&self, candidate: usize, lines: &[usize]) -> usize {
        match self.counterparts.get(&candidate) {
            Some(&(_, slot)) => lines
                .iter()
                .filter(|line| self.maps[slot].added_candidate_lines.contains(line))
                .count(),
            None => lines.len(),
        }
    }
}

/// A candidate occurrence under evaluation: a member of a maximal group in
/// a changed file, with its first other group member.
struct Subject<'a> {
    view: usize,
    run: Candidate,
    added: usize,
    matched: (&'a RepoPath, usize, usize),
}

/// The deleted base occurrences, indexed by token length and window hash.
struct MovePool<'a> {
    entries: Vec<(usize, Candidate, (usize, usize))>,
    index: HashMap<(LanguageFamily, usize, u64), Vec<usize>>,
    lengths: BTreeSet<usize>,
    base_tokens: HashMap<(usize, u32), ContainerTokens<'a>>,
    consumed: HashSet<(usize, usize)>,
}

impl<'a> MovePool<'a> {
    fn new(base_views: &[FileView<'a>], base_runs: &[Vec<Candidate>], facts: &ChangeFacts) -> Self {
        let mut deleted: Vec<(usize, Candidate)> = Vec::new();
        for (&base, state) in &facts.base_states {
            let view = &base_views[base];
            let undeleted: Vec<Vec<usize>> = view
                .containers
                .iter()
                .map(|statements| {
                    let mut prefix = vec![0usize];
                    let mut total = 0usize;
                    for statement in statements {
                        let kept = run_executable_lines(&[statement])
                            .iter()
                            .any(|&line| !facts.is_deleted(state, line));
                        total += usize::from(kept);
                        prefix.push(total);
                    }
                    prefix
                })
                .collect();
            for run in &base_runs[base] {
                let prefix = &undeleted[run.container as usize];
                let first = run.first_statement as usize;
                if prefix[first + run.statement_count] == prefix[first] {
                    deleted.push((base, *run));
                }
            }
        }
        deleted.sort_by_key(|(base, run)| (base_views[*base].path, run.start_line, run.end_line));

        let mut base_tokens: HashMap<(usize, u32), ContainerTokens<'a>> = HashMap::new();
        let mut entries = Vec::with_capacity(deleted.len());
        let mut index: HashMap<(LanguageFamily, usize, u64), Vec<usize>> = HashMap::new();
        let mut lengths = BTreeSet::new();
        for (base, run) in deleted {
            let view = &base_views[base];
            let tokens = base_tokens
                .entry((base, run.container))
                .or_insert_with(|| ContainerTokens::new(view, run.container));
            let (start, end) = tokens.run_range(&run);
            let key = (view.language, end - start, tokens.window_hash(start, end));
            index.entry(key).or_default().push(entries.len());
            lengths.insert(end - start);
            entries.push((base, run, (start, end)));
        }
        MovePool {
            entries,
            index,
            lengths,
            base_tokens,
            consumed: HashSet::new(),
        }
    }

    /// Pairs `run` with the largest eligible unconsumed deleted occurrence
    /// whose token sequence is contiguous inside it, and consumes that
    /// occurrence's lines. Returns whether `run` is a move.
    fn take_move(
        &mut self,
        base_views: &[FileView<'_>],
        language: LanguageFamily,
        tokens: &ContainerTokens<'_>,
        run: &Candidate,
    ) -> bool {
        let (from, to) = tokens.run_range(run);
        let mut best: Option<usize> = None;
        for &length in &self.lengths {
            if length > to - from {
                break;
            }
            for start in from..=to - length {
                let hash = tokens.window_hash(start, start + length);
                let Some(found) = self.index.get(&(language, length, hash)) else {
                    continue;
                };
                for &slot in found {
                    let (base, base_run, (base_from, base_to)) = &self.entries[slot];
                    if run.source_lines > base_run.source_lines + threshold(base_run.source_lines) {
                        continue;
                    }
                    let beats_best = best.is_none_or(|current| {
                        let current_lines = self.entries[current].1.source_lines;
                        (Reverse(base_run.source_lines), slot) < (Reverse(current_lines), current)
                    });
                    if !beats_best {
                        continue;
                    }
                    let same_tokens = self
                        .base_tokens
                        .get(&(*base, base_run.container))
                        .is_some_and(|base_tokens| {
                            base_tokens.window(*base_from, *base_to)
                                == tokens.window(start, start + length)
                        });
                    let lines = base_views[*base].executable_lines(base_run);
                    if same_tokens
                        && lines
                            .iter()
                            .all(|&line| !self.consumed.contains(&(*base, line)))
                    {
                        best = Some(slot);
                    }
                }
            }
        }
        let Some(slot) = best else {
            return false;
        };
        let (base, base_run, _) = self.entries[slot];
        for line in base_views[base].executable_lines(&base_run) {
            self.consumed.insert((base, line));
        }
        true
    }
}

/// Per statement count `1..=max_count` of the run starting at `first`: how
/// many of its executable lines are in `mapped_lines`, and whether any maps
/// through `base_to_candidate` outside `span`. `statement_lines` holds each
/// statement's own D11 lines; a line a previous statement of the run already
/// reached is counted once, as `run_executable_lines` does.
fn run_tallies(
    statement_lines: &[Vec<usize>],
    first: usize,
    max_count: usize,
    mapped_lines: &[usize],
    base_to_candidate: &BTreeMap<usize, usize>,
    span: (usize, usize),
) -> Vec<(usize, bool)> {
    let mut last_counted_line = 0usize;
    let mut overlap = 0usize;
    let mut escapes = false;
    statement_lines[first..first + max_count]
        .iter()
        .map(|lines| {
            for &line in lines {
                if line <= last_counted_line {
                    continue;
                }
                last_counted_line = line;
                overlap += usize::from(mapped_lines.binary_search(&line).is_ok());
                escapes |= base_to_candidate
                    .get(&line)
                    .is_some_and(|mapped| !(span.0..=span.1).contains(mapped));
            }
            (overlap, escapes)
        })
        .collect()
}

/// The base occurrence of the counterpart file that shares the most
/// executable lines with the candidate occurrence through the line map; ties
/// go to the larger one, then source order. A base occurrence with an
/// executable line that maps outside `span` (the candidate occurrence's
/// lines) is not eligible: it encloses more than the occurrence.
fn diff_mapped(
    base: usize,
    base_view: &FileView<'_>,
    base_by_container: &HashMap<(usize, u32), Vec<Candidate>>,
    statement_lines: &mut HashMap<(usize, u32), Vec<Vec<usize>>>,
    base_to_candidate: &BTreeMap<usize, usize>,
    span: (usize, usize),
    mapped_lines: &[usize],
) -> Option<Candidate> {
    let (&lowest, &highest) = (mapped_lines.first()?, mapped_lines.last()?);
    let mut best: Option<(OverlapRank, Candidate)> = None;
    for (container, &(span_start, span_end)) in base_view.spans.iter().enumerate() {
        if span_start > highest || span_end < lowest {
            continue;
        }
        let Some(runs) = base_by_container.get(&(base, container as u32)) else {
            continue;
        };
        let touching: Vec<&Candidate> = runs
            .iter()
            .filter(|run| run.start_line <= highest && run.end_line >= lowest)
            .collect();
        let mut longest: BTreeMap<usize, usize> = BTreeMap::new();
        for run in &touching {
            let count = longest.entry(run.first_statement as usize).or_insert(0);
            *count = (*count).max(run.statement_count);
        }
        let table = statement_lines
            .entry((base, container as u32))
            .or_insert_with(|| {
                base_view.containers[container]
                    .iter()
                    .map(|statement| run_executable_lines(std::slice::from_ref(statement)))
                    .collect()
            });
        let tallies: BTreeMap<usize, Vec<(usize, bool)>> = longest
            .into_iter()
            .map(|(first, count)| {
                let tally = run_tallies(table, first, count, mapped_lines, base_to_candidate, span);
                (first, tally)
            })
            .collect();
        for run in touching {
            let (overlap, escapes) =
                tallies[&(run.first_statement as usize)][run.statement_count - 1];
            if escapes || overlap == 0 {
                continue;
            }
            let rank = (
                overlap,
                run.source_lines,
                Reverse(run.start_line),
                Reverse(run.end_line),
            );
            if best.as_ref().is_none_or(|(current, _)| rank > *current) {
                best = Some((rank, *run));
            }
        }
    }
    best.map(|(_, run)| run)
}

/// Raises V102 for every changed candidate clone occurrence that is new or
/// materially extended and has a qualifying match elsewhere (A3: a deleted
/// base occurrence that maps to an added one is a move, not a regression).
///
/// `base` and `candidate` are the changed files of each side, `unchanged`
/// the included files no change names. Output is sorted by (candidate path
/// bytes, start line, end line); `Warn` reports like `Deny` and `Off` is
/// empty.
pub fn evaluate_clones(
    base: &[FindingFile<'_>],
    candidate: &[FindingFile<'_>],
    unchanged: &[FindingFile<'_>],
    changes: &[Change],
    min_clone_lines: u32,
    policy: &PolicyConfig,
) -> Result<Vec<CloneDiagnostic>, GitError> {
    if policy.nsd_v102 == Severity::Off {
        return Ok(Vec::new());
    }
    let base_views: Vec<FileView<'_>> = base.iter().filter_map(FileView::new).collect();
    let mut views: Vec<FileView<'_>> = candidate.iter().filter_map(FileView::new).collect();
    let changed_count = views.len();
    views.extend(unchanged.iter().filter_map(FileView::new));
    let facts = ChangeFacts::new(changes, &base_views, &views[..changed_count])?;

    let per_file: Vec<(LanguageFamily, PerFileCandidates)> = views
        .par_iter()
        .enumerate()
        .map(|(index, view)| (view.language, view.enumerate(index, min_clone_lines)))
        .collect();
    let base_runs: Vec<Vec<Candidate>> = base_views
        .par_iter()
        .enumerate()
        .map(|(index, view)| {
            view.enumerate(index, min_clone_lines)
                .into_iter()
                .map(|(_, run)| run)
                .collect()
        })
        .collect();

    let mut subjects: Vec<Subject<'_>> = Vec::new();
    for mut group in maximal_groups(per_file) {
        group.members.sort_by_key(|run| {
            (
                views[run.file_index as usize].path,
                run.start_line,
                run.end_line,
            )
        });
        for (position, run) in group.members.iter().enumerate() {
            let view = run.file_index as usize;
            if view >= changed_count {
                continue;
            }
            let Some(other) = group
                .members
                .iter()
                .enumerate()
                .find_map(|(other, run)| (other != position).then_some(run))
            else {
                continue;
            };
            let lines = views[view].executable_lines(run);
            let added = facts.added_executable_lines(view, &lines);
            if added > 0 {
                subjects.push(Subject {
                    view,
                    run: *run,
                    added,
                    matched: (
                        views[other.file_index as usize].path,
                        other.start_line,
                        other.end_line,
                    ),
                });
            }
        }
    }
    subjects.sort_by_key(|subject| {
        (
            views[subject.view].path,
            subject.run.start_line,
            subject.run.end_line,
        )
    });

    let mut base_by_container: HashMap<(usize, u32), Vec<Candidate>> = HashMap::new();
    for (base, runs) in base_runs.iter().enumerate() {
        for run in runs {
            base_by_container
                .entry((base, run.container))
                .or_default()
                .push(*run);
        }
    }

    let mut statement_lines: HashMap<(usize, u32), Vec<Vec<usize>>> = HashMap::new();
    let mut firing: Vec<(&Subject<'_>, Option<usize>)> = Vec::new();
    let mut added_subjects: Vec<&Subject<'_>> = Vec::new();
    for subject in &subjects {
        let mapped = facts
            .counterparts
            .get(&subject.view)
            .and_then(|&(base, slot)| {
                let mut lines: Vec<usize> = facts.candidate_to_base[slot]
                    .range(subject.run.start_line..=subject.run.end_line)
                    .map(|(_, &line)| line)
                    .collect();
                lines.sort_unstable();
                diff_mapped(
                    base,
                    &base_views[base],
                    &base_by_container,
                    &mut statement_lines,
                    &facts.maps[slot].base_to_candidate,
                    (subject.run.start_line, subject.run.end_line),
                    &lines,
                )
            });
        match mapped {
            Some(base_run) => {
                if subject.added > threshold(base_run.source_lines) {
                    firing.push((subject, Some(base_run.source_lines)));
                }
            }
            None => added_subjects.push(subject),
        }
    }

    if !added_subjects.is_empty() {
        let mut pool = MovePool::new(&base_views, &base_runs, &facts);
        let mut candidate_tokens: HashMap<(usize, u32), ContainerTokens<'_>> = HashMap::new();
        for subject in added_subjects {
            let view = &views[subject.view];
            let tokens = candidate_tokens
                .entry((subject.view, subject.run.container))
                .or_insert_with(|| ContainerTokens::new(view, subject.run.container));
            if !pool.take_move(&base_views, view.language, tokens, &subject.run) {
                firing.push((subject, None));
            }
        }
    }

    let mut sorted: BTreeMap<(&RepoPath, usize, usize), CloneDiagnostic> = BTreeMap::new();
    for (subject, base_lines) in firing {
        let path = views[subject.view].path;
        sorted.insert(
            (path, subject.run.start_line, subject.run.end_line),
            CloneDiagnostic {
                code: CODE_CLONE_REGRESSION,
                candidate_path: path.clone(),
                candidate_start_line: subject.run.start_line,
                candidate_end_line: subject.run.end_line,
                base_lines,
                added_lines: subject.added,
                matched_path: subject.matched.0.clone(),
                matched_start_line: subject.matched.1,
                matched_end_line: subject.matched.2,
            },
        );
    }
    Ok(sorted.into_values().collect())
}

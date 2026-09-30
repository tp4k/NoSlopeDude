//! M3-2: diff-aware finding matching and V101 (`nsd-plan-final.md`
//! *Stable data model*, *Diagnostics*).
//!
//! A finding's key is (rule ID, context, normalized-syntax digest). The
//! context is the matched callable pair when the finding's innermost
//! enclosing callable is matched, else the file, with a base path mapped to
//! its renamed candidate path. Within one key, a base and a candidate
//! occurrence first pair when the diff maps the base start line to the
//! candidate start line; the remaining occurrences then pair k-th to k-th in
//! source order, and only surplus candidates are unmatched. Nothing is
//! persisted and no ordinal enters any digest.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::analysis::FileAnalysis;
use crate::config::{PolicyConfig, Severity};
use crate::git::diff::{map_lines, Change, LineMap};
use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::identity::matching::MatchOutput;
use crate::model::RuleId;
use crate::policy::diagnostics::{FindingDiagnostic, CODE_UNMATCHED_FINDING};

/// One snapshot file's bytes and analysis; `analysis.callables` is
/// index-aligned with the `FileCallables` the callable matcher saw.
pub struct FindingFile<'a> {
    pub path: RepoPath,
    pub source: &'a [u8],
    pub analysis: &'a FileAnalysis,
}

/// One finding: its file and its index in `FileAnalysis::findings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRef {
    pub path: RepoPath,
    pub index: usize,
}

/// A matched base/candidate finding pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingPair {
    pub base: FindingRef,
    pub candidate: FindingRef,
}

/// The pairs, the candidate findings left unpaired, and their V101s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindingMatchOutput {
    pub pairs: Vec<FindingPair>,
    pub unmatched_candidates: Vec<FindingRef>,
    pub diagnostics: Vec<FindingDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Context<'a> {
    Callable(&'a RepoPath, usize),
    File(&'a RepoPath),
}

type Key<'a> = (RuleId, Context<'a>, &'a str);

#[derive(Clone, Copy)]
struct Occurrence<'a> {
    path: &'a RepoPath,
    index: usize,
    rule_id: RuleId,
    start_line: usize,
    end_line: usize,
}

#[derive(Default)]
struct Group<'a> {
    base: Vec<Occurrence<'a>>,
    candidate: Vec<Occurrence<'a>>,
}

fn occurrence<'a>(file: &'a FindingFile<'_>, index: usize) -> Occurrence<'a> {
    let finding = &file.analysis.findings[index].finding;
    Occurrence {
        path: &file.path,
        index,
        rule_id: finding.rule_id,
        start_line: finding.start_line,
        end_line: finding.end_line,
    }
}

fn finding_ref(found: &Occurrence<'_>) -> FindingRef {
    FindingRef {
        path: found.path.clone(),
        index: found.index,
    }
}

/// Matches `base` findings to `candidate` findings and raises V101 for
/// every unmatched candidate finding. `base` and `candidate` are the changed
/// files of each side; `matched` and `changes` come from the callable
/// matcher and the diff. Output is sorted by (candidate path bytes, finding
/// index).
pub fn match_findings<'a>(
    base: &'a [FindingFile<'_>],
    candidate: &'a [FindingFile<'_>],
    matched: &'a MatchOutput,
    changes: &'a [Change],
    policy: &PolicyConfig,
) -> Result<FindingMatchOutput, GitError> {
    let renames: HashMap<&RepoPath, &RepoPath> = changes
        .iter()
        .filter_map(|change| match change {
            Change::Renamed { from, to, .. } => Some((from, to)),
            _ => None,
        })
        .collect();
    let counterpart = |path: &'a RepoPath| renames.get(path).copied().unwrap_or(path);
    let callable_pairs: HashMap<(&RepoPath, usize), (&RepoPath, usize)> = matched
        .matches
        .iter()
        .map(|found| {
            (
                (&found.base.path, found.base.index),
                (&found.candidate.path, found.candidate.index),
            )
        })
        .collect();
    let matched_candidates: HashSet<(&RepoPath, usize)> =
        callable_pairs.values().copied().collect();

    let mut groups: HashMap<Key<'a>, Group<'a>> = HashMap::new();
    for file in base {
        for (index, analyzed) in file.analysis.findings.iter().enumerate() {
            let context = analyzed
                .enclosing_callable
                .and_then(|callable| callable_pairs.get(&(&file.path, callable)))
                .map_or_else(
                    || Context::File(counterpart(&file.path)),
                    |&(path, callable)| Context::Callable(path, callable),
                );
            let key = (
                analyzed.finding.rule_id,
                context,
                analyzed.syntax_digest.as_str(),
            );
            groups
                .entry(key)
                .or_default()
                .base
                .push(occurrence(file, index));
        }
    }
    for file in candidate {
        for (index, analyzed) in file.analysis.findings.iter().enumerate() {
            let context = match analyzed.enclosing_callable {
                Some(callable) if matched_candidates.contains(&(&file.path, callable)) => {
                    Context::Callable(&file.path, callable)
                }
                _ => Context::File(&file.path),
            };
            let key = (
                analyzed.finding.rule_id,
                context,
                analyzed.syntax_digest.as_str(),
            );
            groups
                .entry(key)
                .or_default()
                .candidate
                .push(occurrence(file, index));
        }
    }

    let candidate_by_path: HashMap<&RepoPath, &FindingFile<'_>> =
        candidate.iter().map(|file| (&file.path, file)).collect();
    let mut line_maps: HashMap<&RepoPath, LineMap> = HashMap::new();
    for file in base {
        let counter = candidate_by_path.get(counterpart(&file.path));
        if let Some(counter) = counter.filter(|counter| {
            !file.analysis.findings.is_empty() && !counter.analysis.findings.is_empty()
        }) {
            line_maps.insert(&file.path, map_lines(file.source, counter.source)?);
        }
    }

    let mut pairs = Vec::new();
    let mut unmatched = Vec::new();
    for group in groups.values_mut() {
        let order = |found: &Occurrence<'a>| (found.path, found.index);
        group.base.sort_by_key(order);
        group.candidate.sort_by_key(order);

        let mut candidate_by_line: HashMap<(&RepoPath, usize), VecDeque<usize>> = HashMap::new();
        for (position, found) in group.candidate.iter().enumerate() {
            candidate_by_line
                .entry((found.path, found.start_line))
                .or_default()
                .push_back(position);
        }
        let mut taken = vec![false; group.candidate.len()];
        let mut open_base = Vec::new();
        for found in &group.base {
            let target = line_maps
                .get(found.path)
                .and_then(|line_map| line_map.base_to_candidate.get(&found.start_line))
                .and_then(|&line| candidate_by_line.get_mut(&(counterpart(found.path), line)))
                .and_then(VecDeque::pop_front);
            match target {
                Some(position) => {
                    taken[position] = true;
                    pairs.push(FindingPair {
                        base: finding_ref(found),
                        candidate: finding_ref(&group.candidate[position]),
                    });
                }
                None => open_base.push(found),
            }
        }
        let mut open_candidate = group
            .candidate
            .iter()
            .zip(&taken)
            .filter(|(_, &is_taken)| !is_taken)
            .map(|(found, _)| found);
        for found in open_base {
            if let Some(paired) = open_candidate.next() {
                pairs.push(FindingPair {
                    base: finding_ref(found),
                    candidate: finding_ref(paired),
                });
            }
        }
        unmatched.extend(open_candidate.copied());
    }

    pairs.sort_by(|a, b| {
        (&a.candidate.path, a.candidate.index).cmp(&(&b.candidate.path, b.candidate.index))
    });
    unmatched.sort_by_key(|found| (found.path, found.index));
    let diagnostics = if policy.nsd_v101 == Severity::Off {
        Vec::new()
    } else {
        unmatched
            .iter()
            .map(|found| FindingDiagnostic {
                code: CODE_UNMATCHED_FINDING,
                rule_id: found.rule_id,
                candidate_path: found.path.clone(),
                candidate_start_line: found.start_line,
                candidate_end_line: found.end_line,
            })
            .collect()
    };
    Ok(FindingMatchOutput {
        pairs,
        unmatched_candidates: unmatched.iter().map(finding_ref).collect(),
        diagnostics,
    })
}

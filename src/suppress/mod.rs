//! M3-3: source suppressions (`nsd-plan-final.md` *Stable data model*,
//! *Diagnostics*, A9).
//!
//! A valid directive suppresses the finding of its rule that starts on the
//! next line. Suppression is applied after finding matching: S101 is raised
//! for a suppressed candidate finding unless its matched base finding was
//! suppressed too, and S102 is delta-based against the base directive the
//! diff maps each candidate directive to.

mod directive;

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::config::{PolicyConfig, Severity};
use crate::git::diff::{map_lines, Change};
use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::model::RuleId;
use crate::policy::diagnostics::{
    FindingDiagnostic, SuppressionDiagnostic, CODE_INVALID_SUPPRESSION, CODE_NEW_SUPPRESSION,
};
use crate::policy::findings::{FindingFile, FindingMatchOutput, FindingRef};

use directive::{directives, Directive, Form};

/// The S101/S102 diagnostics, and the V101s left after suppression.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuppressionOutput {
    pub diagnostics: Vec<SuppressionDiagnostic>,
    pub findings: Vec<FindingDiagnostic>,
}

/// One file's directives, which are used, and which of its findings they
/// suppress (indices into `FileAnalysis::findings`).
struct FileState {
    directives: Vec<Directive>,
    used: Vec<bool>,
    suppressed: HashSet<usize>,
}

impl FileState {
    fn of(file: &FindingFile<'_>) -> FileState {
        let directives = directives(&file.analysis.ir.root, file.source);
        let mut used = vec![false; directives.len()];
        let mut suppressed = HashSet::new();
        let mut by_site: HashMap<(RuleId, usize), Vec<usize>> = HashMap::new();
        for (index, analyzed) in file.analysis.findings.iter().enumerate() {
            let finding = &analyzed.finding;
            by_site
                .entry((finding.rule_id, finding.start_line))
                .or_default()
                .push(index);
        }
        for (slot, directive) in directives.iter().enumerate() {
            let Form::Valid(rule) = directive.form else {
                continue;
            };
            if let Some(indices) = by_site.get(&(rule, directive.line + 1)) {
                used[slot] = true;
                suppressed.extend(indices);
            }
        }
        FileState {
            directives,
            used,
            suppressed,
        }
    }

    fn is_problem(&self, slot: usize) -> bool {
        self.directives[slot].form == Form::Invalid || !self.used[slot]
    }

    /// Slots of the directives on `line`; `directives` is sorted by line.
    fn slots_at(&self, line: usize) -> Range<usize> {
        let first = self.directives.partition_point(|d| d.line < line);
        let last = self.directives.partition_point(|d| d.line <= line);
        first..last
    }
}

fn s102(path: &RepoPath, directive: &Directive) -> SuppressionDiagnostic {
    SuppressionDiagnostic {
        code: CODE_INVALID_SUPPRESSION,
        rule_id: match directive.form {
            Form::Valid(rule) => Some(rule),
            Form::Invalid => None,
        },
        candidate_path: path.clone(),
        directive_line: directive.line,
    }
}

fn sort_diagnostics(diagnostics: &mut [SuppressionDiagnostic]) {
    diagnostics.sort_by(|a, b| {
        (&a.candidate_path, a.directive_line, a.code).cmp(&(
            &b.candidate_path,
            b.directive_line,
            b.code,
        ))
    });
}

/// Applies suppressions to a matched base/candidate pair. `base` and
/// `candidate` are the changed files of each side, as given to
/// `match_findings`; `matched` is its output and `changes` the diff. An
/// `Err` from the line map fails closed.
pub fn apply_suppressions(
    base: &[FindingFile<'_>],
    candidate: &[FindingFile<'_>],
    matched: &FindingMatchOutput,
    changes: &[Change],
    policy: &PolicyConfig,
) -> Result<SuppressionOutput, GitError> {
    let base_files: HashMap<&RepoPath, (&FindingFile<'_>, FileState)> = base
        .iter()
        .map(|file| (&file.path, (file, FileState::of(file))))
        .collect();
    let candidate_states: Vec<FileState> = candidate.iter().map(FileState::of).collect();
    let base_of: HashMap<&RepoPath, &RepoPath> = changes
        .iter()
        .filter_map(|change| match change {
            Change::Renamed { from, to, .. } => Some((to, from)),
            _ => None,
        })
        .collect();
    let base_partner: HashMap<(&RepoPath, usize), &FindingRef> = matched
        .pairs
        .iter()
        .map(|pair| ((&pair.candidate.path, pair.candidate.index), &pair.base))
        .collect();

    let mut diagnostics = Vec::new();
    let mut suppressed_lines: HashSet<(&RepoPath, RuleId, usize)> = HashSet::new();
    for (file, state) in candidate.iter().zip(&candidate_states) {
        for &index in &state.suppressed {
            let finding = &file.analysis.findings[index].finding;
            suppressed_lines.insert((&file.path, finding.rule_id, finding.start_line));
            let base_suppressed = base_partner
                .get(&(&file.path, index))
                .and_then(|base_ref| {
                    base_files
                        .get(&base_ref.path)
                        .map(|(_, state)| state.suppressed.contains(&base_ref.index))
                })
                .unwrap_or(false);
            if !base_suppressed {
                diagnostics.push(SuppressionDiagnostic {
                    code: CODE_NEW_SUPPRESSION,
                    rule_id: Some(finding.rule_id),
                    candidate_path: file.path.clone(),
                    directive_line: finding.start_line - 1,
                });
            }
        }
    }

    if policy.nsd_s102 != Severity::Off {
        for (file, state) in candidate.iter().zip(&candidate_states) {
            let problems: Vec<usize> = (0..state.directives.len())
                .filter(|&slot| state.is_problem(slot))
                .collect();
            if problems.is_empty() {
                continue;
            }
            let base_path = base_of.get(&file.path).copied().unwrap_or(&file.path);
            let twin = base_files.get(base_path);
            let candidate_to_base: HashMap<usize, usize> = match twin {
                Some((base_file, _)) => map_lines(base_file.source, file.source)?
                    .base_to_candidate
                    .into_iter()
                    .map(|(from, to)| (to, from))
                    .collect(),
                None => HashMap::new(),
            };
            for slot in problems {
                let current = &state.directives[slot];
                let tolerated = twin.is_some_and(|(_, base_state)| {
                    candidate_to_base.get(&current.line).is_some_and(|&line| {
                        base_state.slots_at(line).any(|base_slot| {
                            base_state.directives[base_slot].text == current.text
                                && base_state.is_problem(base_slot)
                        })
                    })
                });
                if !tolerated {
                    diagnostics.push(s102(&file.path, current));
                }
            }
        }
    }
    sort_diagnostics(&mut diagnostics);

    let findings = matched
        .diagnostics
        .iter()
        .filter(|found| {
            !suppressed_lines.contains(&(
                &found.candidate_path,
                found.rule_id,
                found.candidate_start_line,
            ))
        })
        .cloned()
        .collect();
    Ok(SuppressionOutput {
        diagnostics,
        findings,
    })
}

/// One-snapshot semantics: every invalid and unused directive.
pub fn scan_suppressions(
    files: &[FindingFile<'_>],
    policy: &PolicyConfig,
) -> Vec<SuppressionDiagnostic> {
    if policy.nsd_s102 == Severity::Off {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for file in files {
        let state = FileState::of(file);
        for (slot, current) in state.directives.iter().enumerate() {
            if state.is_problem(slot) {
                diagnostics.push(s102(&file.path, current));
            }
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

//! M3-1: E101/E102 classification of matched callables, and G102 for a
//! callable-match ambiguity that could change a verdict (`nsd-plan-final.md`
//! *Diagnostics*, *Stable data model*).
//!
//! A candidate's verdict depends on its paired base callable (or on none).
//! Where the matcher could not pick one base callable -- a tier-3 fingerprint
//! bucket, or a same-key group's positional remainder -- the candidate's
//! options are every base member of that set, plus "unmatched" when the set
//! has more candidates than base callables. G102 fires when those options
//! disagree; when they agree, the common verdict is emitted. The options are
//! summarized once per set in a `BaseSet`, so each candidate costs
//! O(log base), not O(base).

use std::collections::HashMap;

use crate::config::{PolicyConfig, Severity};
use crate::git::path::RepoPath;
use crate::identity::matching::{CallableRef, MatchOutput, MatchPairing};
use crate::model::Callable;
use crate::policy::diagnostics::{
    BaseCallable, PolicyDiagnostic, CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_COMPLEXITY_INCREASED,
    CODE_MATCH_AMBIGUITY,
};

/// The immutable policy CC threshold (`nsd-plan-final.md` *CLI and
/// configuration*); unrelated to the D13 erosion-mass cut.
const POLICY_CC_THRESHOLD: u32 = 10;
/// E102's SLOC growth allowance is `max(SLOC_GROWTH_FLOOR,
/// floor(base_sloc / SLOC_GROWTH_DIVISOR))`.
const SLOC_GROWTH_FLOOR: usize = 1;
const SLOC_GROWTH_DIVISOR: usize = 10;

/// One file's callable metrics from one snapshot, index-aligned with the
/// `FileCallables` the matcher saw for the same path. A ref into a path or
/// index missing here is skipped.
#[derive(Debug, Clone)]
pub struct FileMetrics {
    pub path: RepoPath,
    pub callables: Vec<Callable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    E101,
    E102,
}

/// The base SLOC above which a candidate's SLOC counts as E102 growth.
fn growth_limit(base_sloc: usize) -> usize {
    base_sloc + SLOC_GROWTH_FLOOR.max(base_sloc / SLOC_GROWTH_DIVISOR)
}

/// The spec's verdict of one candidate against one base callable, or none,
/// before any `off` code is applied.
fn raw_verdict(candidate: &Callable, base: Option<&Callable>) -> Verdict {
    if candidate.cc <= POLICY_CC_THRESHOLD {
        return Verdict::Pass;
    }
    match base {
        Some(base) if base.cc > POLICY_CC_THRESHOLD => {
            if candidate.cc > base.cc || candidate.sloc > growth_limit(base.sloc) {
                Verdict::E102
            } else {
                Verdict::Pass
            }
        }
        _ => Verdict::E101,
    }
}

/// A code set to `off` counts as a pass.
fn effective(verdict: Verdict, policy: &PolicyConfig) -> Verdict {
    let severity = match verdict {
        Verdict::Pass => return Verdict::Pass,
        Verdict::E101 => policy.nsd_e101,
        Verdict::E102 => policy.nsd_e102,
    };
    if severity == Severity::Off {
        Verdict::Pass
    } else {
        verdict
    }
}

/// What every candidate in an ambiguity set has to choose between, reduced
/// to the few numbers that decide each verdict kind.
struct BaseSet {
    has_at_or_below_threshold: bool,
    min_cc_above_threshold: Option<u32>,
    min_growth_limit_above_threshold: Option<usize>,
    /// Base callables above the threshold, ascending by CC, each with the
    /// largest growth limit among it and every later (higher-CC) entry.
    suffix_max_limit: Vec<(u32, usize)>,
}

impl BaseSet {
    fn new<'a>(members: impl Iterator<Item = &'a Callable>) -> BaseSet {
        let mut has_at_or_below_threshold = false;
        let mut high: Vec<(u32, usize)> = Vec::new();
        for member in members {
            if member.cc > POLICY_CC_THRESHOLD {
                high.push((member.cc, growth_limit(member.sloc)));
            } else {
                has_at_or_below_threshold = true;
            }
        }
        high.sort_unstable();
        let min_cc_above_threshold = high.first().map(|&(cc, _)| cc);
        let min_growth_limit_above_threshold = high.iter().map(|&(_, limit)| limit).min();
        let mut running = 0;
        for entry in high.iter_mut().rev() {
            running = running.max(entry.1);
            entry.1 = running;
        }
        BaseSet {
            has_at_or_below_threshold,
            min_cc_above_threshold,
            min_growth_limit_above_threshold,
            suffix_max_limit: high,
        }
    }

    /// The one verdict every option of `candidate` gives, or `None` when the
    /// options disagree. `unmatched_possible` adds the "no base" option.
    fn common_verdict(
        &self,
        candidate: &Callable,
        unmatched_possible: bool,
        policy: &PolicyConfig,
    ) -> Option<Verdict> {
        if candidate.cc <= POLICY_CC_THRESHOLD {
            return Some(Verdict::Pass);
        }
        let mut seen = Vec::with_capacity(3);
        let mut note = |verdict: Verdict| {
            let verdict = effective(verdict, policy);
            if !seen.contains(&verdict) {
                seen.push(verdict);
            }
        };
        if self.has_at_or_below_threshold || unmatched_possible {
            note(Verdict::E101);
        }
        let grows = self
            .min_cc_above_threshold
            .is_some_and(|cc| candidate.cc > cc)
            || self
                .min_growth_limit_above_threshold
                .is_some_and(|limit| candidate.sloc > limit);
        if grows {
            note(Verdict::E102);
        }
        let first_not_lower = self
            .suffix_max_limit
            .partition_point(|&(cc, _)| cc < candidate.cc);
        let passes = self
            .suffix_max_limit
            .get(first_not_lower)
            .is_some_and(|&(_, limit)| candidate.sloc <= limit);
        if passes {
            note(Verdict::Pass);
        }
        match seen.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }
}

type Keyed = ((RepoPath, usize), PolicyDiagnostic);

struct Classifier<'a> {
    base: HashMap<&'a RepoPath, &'a [Callable]>,
    candidate: HashMap<&'a RepoPath, &'a [Callable]>,
    policy: &'a PolicyConfig,
    out: Vec<Keyed>,
}

impl<'a> Classifier<'a> {
    fn base_callable(&self, callable_ref: &CallableRef) -> Option<&'a Callable> {
        self.base.get(&callable_ref.path)?.get(callable_ref.index)
    }

    fn candidate_callable(&self, callable_ref: &'a CallableRef) -> Option<&'a Callable> {
        self.candidate
            .get(&callable_ref.path)?
            .get(callable_ref.index)
    }

    fn base_set(&self, members: &[CallableRef]) -> BaseSet {
        BaseSet::new(
            members
                .iter()
                .filter_map(|member| self.base_callable(member)),
        )
    }

    fn push(
        &mut self,
        code: &'static str,
        candidate_ref: &CallableRef,
        candidate: &Callable,
        base: Option<&CallableRef>,
    ) {
        let base = base.and_then(|base_ref| {
            self.base_callable(base_ref).map(|base| BaseCallable {
                path: base_ref.path.clone(),
                start_line: base.start_line,
                end_line: base.end_line,
                cc: base.cc,
                sloc: base.sloc,
            })
        });
        self.out.push((
            (candidate_ref.path.clone(), candidate_ref.index),
            PolicyDiagnostic {
                code,
                candidate_path: candidate_ref.path.clone(),
                candidate_name: candidate.name.clone(),
                candidate_start_line: candidate.start_line,
                candidate_end_line: candidate.end_line,
                base,
            },
        ));
    }

    /// Emits the verdict of a candidate paired with `base` (or unmatched).
    fn pair(&mut self, candidate_ref: &CallableRef, base: Option<&CallableRef>) {
        let Some(candidate) = self.candidate_callable(candidate_ref) else {
            return;
        };
        let base_callable = base.and_then(|base_ref| self.base_callable(base_ref));
        let code = match effective(raw_verdict(candidate, base_callable), self.policy) {
            Verdict::Pass => return,
            Verdict::E101 => CODE_COMPLEXITY_ABOVE_THRESHOLD,
            Verdict::E102 => CODE_COMPLEXITY_INCREASED,
        };
        self.push(code, candidate_ref, candidate, base);
    }

    /// Emits G102 when `candidate`'s options over `set` disagree, else the
    /// common verdict with `designated` (the pairing already made or implied)
    /// filling the base fields.
    fn decide(
        &mut self,
        candidate_ref: &CallableRef,
        set: &BaseSet,
        unmatched_possible: bool,
        designated: Option<&CallableRef>,
    ) {
        let Some(candidate) = self.candidate_callable(candidate_ref) else {
            return;
        };
        let code = match set.common_verdict(candidate, unmatched_possible, self.policy) {
            None => {
                self.push(CODE_MATCH_AMBIGUITY, candidate_ref, candidate, None);
                return;
            }
            Some(Verdict::Pass) => return,
            Some(Verdict::E101) => CODE_COMPLEXITY_ABOVE_THRESHOLD,
            Some(Verdict::E102) => CODE_COMPLEXITY_INCREASED,
        };
        self.push(code, candidate_ref, candidate, designated);
    }
}

/// Classifies every candidate callable against `matched`. Unmatched base
/// callables (deletions) produce nothing. Diagnostics are sorted by
/// (candidate path bytes, candidate callable index).
pub fn classify(
    base: &[FileMetrics],
    candidate: &[FileMetrics],
    matched: &MatchOutput,
    policy: &PolicyConfig,
) -> Vec<PolicyDiagnostic> {
    let mut classifier = Classifier {
        base: base
            .iter()
            .map(|file| (&file.path, file.callables.as_slice()))
            .collect(),
        candidate: candidate
            .iter()
            .map(|file| (&file.path, file.callables.as_slice()))
            .collect(),
        policy,
        out: Vec::new(),
    };
    let mut handled: HashMap<&RepoPath, Vec<bool>> = candidate
        .iter()
        .map(|file| (&file.path, vec![false; file.callables.len()]))
        .collect();
    let mut mark = |callable_ref: &CallableRef| {
        if let Some(flag) = handled
            .get_mut(&callable_ref.path)
            .and_then(|flags| flags.get_mut(callable_ref.index))
        {
            *flag = true;
        }
    };

    let mut positional: HashMap<(&RepoPath, usize), &CallableRef> = HashMap::new();
    for found in &matched.matches {
        mark(&found.candidate);
        match found.pairing {
            MatchPairing::FingerprintExact => {
                classifier.pair(&found.candidate, Some(&found.base));
            }
            MatchPairing::Positional => {
                positional.insert((&found.candidate.path, found.candidate.index), &found.base);
            }
        }
    }

    for remainder in &matched.positional_remainders {
        let set = classifier.base_set(&remainder.base);
        let unmatched_possible = remainder.candidate.len() > remainder.base.len();
        for candidate_ref in &remainder.candidate {
            if let Some(paired) = positional.remove(&(&candidate_ref.path, candidate_ref.index)) {
                classifier.decide(candidate_ref, &set, unmatched_possible, Some(paired));
            }
        }
    }
    for ((path, index), paired) in positional {
        let candidate_ref = CallableRef {
            path: path.clone(),
            index,
        };
        classifier.pair(&candidate_ref, Some(paired));
    }

    for ambiguity in &matched.ambiguities {
        let set = classifier.base_set(&ambiguity.base);
        let unmatched_possible = ambiguity.candidate.len() > ambiguity.base.len();
        for (position, candidate_ref) in ambiguity.candidate.iter().enumerate() {
            mark(candidate_ref);
            classifier.decide(
                candidate_ref,
                &set,
                unmatched_possible,
                ambiguity.base.get(position),
            );
        }
    }

    let unmatched: Vec<CallableRef> = candidate
        .iter()
        .flat_map(|file| {
            let flags = &handled[&file.path];
            (0..file.callables.len())
                .filter(|&index| !flags[index])
                .map(|index| CallableRef {
                    path: file.path.clone(),
                    index,
                })
        })
        .collect();
    for candidate_ref in &unmatched {
        classifier.pair(candidate_ref, None);
    }

    let mut out = classifier.out;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, diagnostic)| diagnostic).collect()
}

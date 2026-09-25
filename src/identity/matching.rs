//! M2-3: matches callables across two snapshots by the spec's three tiers, in
//! order (`nsd-plan-final.md` *Stable data model*): (1) same path, equal
//! `CallableIdentity`; (2) a `git::diff::Change::Renamed` pair, equal
//! identity; (3) an exactly-1:1 leftover body fingerprint. A prerequisite for
//! E101/E102 (M3) -- nothing here classifies a diff, emits a diagnostic, or
//! wires into `pipeline.rs`; that is the next milestone's job (decision 13).
//!
//! Round 1 (this commit): tier 1 only. Tiers 2-3 and ambiguity land in later
//! commits of the same round (strict TDD, one behaviour at a time).
//!
//! Every input is already computed by the caller: WS-1's `identity::
//! identities` zipped with `identity::body_fingerprint` for the
//! `(CallableIdentity, fingerprint)` pairs, `git::diff`'s own `Change` list
//! for tier 2. This module never calls either itself -- WS-1's own triage
//! notes flagged the cost of materializing an owner chain more than once per
//! callable, and a per-file `identities()` call here would do exactly that.
//!
//! Same-key grouping (tier 1 and, later, tier 2 within a renamed pair)
//! follows the finding-matching rule the spec's *Stable data model* names
//! for a repeated identical finding: pair equal fingerprints first, in
//! source order (the k-th repeated fingerprint on one side pairs with the
//! k-th on the other), then pair whatever remains by order-preserving greedy
//! matching in source order. Every group is built with a hash map, never by
//! comparing every base callable against every candidate one.

use std::collections::{HashMap, HashSet};

use crate::git::diff::Change;
use crate::git::path::RepoPath;
use crate::identity::CallableIdentity;

/// One file's callables from one snapshot, in document order -- WS-1's
/// `identity::identities` zipped with a `identity::body_fingerprint` call per
/// callable, computed once by the caller before matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCallables {
    pub path: RepoPath,
    pub callables: Vec<(CallableIdentity, String)>,
}

/// One callable in one snapshot: its file and its position in that file's
/// `FileCallables::callables` (document order) -- the two coordinates needed
/// to look it back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableRef {
    pub path: RepoPath,
    pub index: usize,
}

/// Which of the spec's three tiers produced a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTier {
    /// Tier 1: same path, equal `CallableIdentity`.
    Structural,
    /// Tier 2: a `Change::Renamed` pair, equal `CallableIdentity`.
    Rename,
    /// Tier 3: an exactly-1:1 leftover body fingerprint.
    BodyFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableMatch {
    pub base: CallableRef,
    pub candidate: CallableRef,
    pub tier: MatchTier,
}

/// A tier-3 fingerprint bucket with more than one leftover callable on
/// either side: nothing in `base` matches anything in `candidate` (either
/// list may be empty). Not yet produced (tier 3 is a later commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguity {
    pub fingerprint: String,
    pub base: Vec<CallableRef>,
    pub candidate: Vec<CallableRef>,
}

/// Deterministic matching output: `matches` sorted by (base path bytes, base
/// callable index), and each `Ambiguity`'s own two lists sorted the same way,
/// with `ambiguities` itself sorted by fingerprint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchOutput {
    pub matches: Vec<CallableMatch>,
    pub ambiguities: Vec<Ambiguity>,
}

/// Matches `base`'s callables against `candidate`'s. Round 1: tier 1 only
/// (same path, equal identity); `changes` is accepted now (tier 2's future
/// signature) but not yet consulted.
pub fn match_callables(
    base: &[FileCallables],
    candidate: &[FileCallables],
    _changes: &[Change],
) -> MatchOutput {
    let candidate_by_path: HashMap<&RepoPath, usize> = candidate
        .iter()
        .enumerate()
        .map(|(index, file)| (&file.path, index))
        .collect();

    let mut matches = Vec::new();

    // Tier 1: same path, equal identity.
    for base_file in base.iter() {
        let Some(&candidate_index) = candidate_by_path.get(&base_file.path) else {
            continue;
        };
        let candidate_file = &candidate[candidate_index];
        let pairs = group_and_pair(
            &base_file.callables,
            0..base_file.callables.len(),
            &candidate_file.callables,
            0..candidate_file.callables.len(),
        );
        record_matches(
            &pairs,
            base_file,
            candidate_file,
            MatchTier::Structural,
            &mut matches,
        );
    }

    matches.sort_by(|a, b| ref_order(&a.base).cmp(&ref_order(&b.base)));

    MatchOutput {
        matches,
        ambiguities: Vec::new(),
    }
}

/// The (path, index) sort key the spec's Output bullet names.
fn ref_order(callable_ref: &CallableRef) -> (&RepoPath, usize) {
    (&callable_ref.path, callable_ref.index)
}

/// Groups `base_indices`/`candidate_indices` (local indices into their own
/// file's callables, in document order) by equal `CallableIdentity`, then
/// pairs each side's same-key group with `pair_group`. Hash-keyed: an
/// intersection of two hash maps, never an O(n*m) comparison of every base
/// callable against every candidate one.
fn group_and_pair(
    base_callables: &[(CallableIdentity, String)],
    base_indices: impl Iterator<Item = usize>,
    candidate_callables: &[(CallableIdentity, String)],
    candidate_indices: impl Iterator<Item = usize>,
) -> Vec<(usize, usize)> {
    let base_groups = identity_groups(base_callables, base_indices);
    let candidate_groups = identity_groups(candidate_callables, candidate_indices);

    let mut pairs = Vec::new();
    for (identity, base_group) in &base_groups {
        let Some(candidate_group) = candidate_groups.get(identity) else {
            continue;
        };
        pairs.extend(pair_group(
            base_group,
            base_callables,
            candidate_group,
            candidate_callables,
        ));
    }
    pairs
}

fn identity_groups(
    callables: &[(CallableIdentity, String)],
    indices: impl Iterator<Item = usize>,
) -> HashMap<&CallableIdentity, Vec<usize>> {
    let mut groups: HashMap<&CallableIdentity, Vec<usize>> = HashMap::new();
    for index in indices {
        groups.entry(&callables[index].0).or_default().push(index);
    }
    groups
}

/// One same-key group's own pairing (this module's own doc comment): equal
/// fingerprints pair first, in source order; then whatever remains pairs by
/// order-preserving greedy matching in source order. `base_indices`/
/// `candidate_indices` must already be in ascending (document) order; any
/// surplus on the larger side is left unmatched.
fn pair_group(
    base_indices: &[usize],
    base_callables: &[(CallableIdentity, String)],
    candidate_indices: &[usize],
    candidate_callables: &[(CallableIdentity, String)],
) -> Vec<(usize, usize)> {
    let mut base_by_fingerprint: HashMap<&str, Vec<usize>> = HashMap::new();
    for &index in base_indices {
        base_by_fingerprint
            .entry(base_callables[index].1.as_str())
            .or_default()
            .push(index);
    }
    let mut candidate_by_fingerprint: HashMap<&str, Vec<usize>> = HashMap::new();
    for &index in candidate_indices {
        candidate_by_fingerprint
            .entry(candidate_callables[index].1.as_str())
            .or_default()
            .push(index);
    }

    let mut pairs = Vec::new();
    let mut consumed_base: HashSet<usize> = HashSet::new();
    let mut consumed_candidate: HashSet<usize> = HashSet::new();
    for (fingerprint, base_bucket) in &base_by_fingerprint {
        let Some(candidate_bucket) = candidate_by_fingerprint.get(fingerprint) else {
            continue;
        };
        let paired = base_bucket.len().min(candidate_bucket.len());
        for k in 0..paired {
            pairs.push((base_bucket[k], candidate_bucket[k]));
            consumed_base.insert(base_bucket[k]);
            consumed_candidate.insert(candidate_bucket[k]);
        }
    }

    let remaining_base: Vec<usize> = base_indices
        .iter()
        .copied()
        .filter(|index| !consumed_base.contains(index))
        .collect();
    let remaining_candidate: Vec<usize> = candidate_indices
        .iter()
        .copied()
        .filter(|index| !consumed_candidate.contains(index))
        .collect();
    let paired = remaining_base.len().min(remaining_candidate.len());
    for k in 0..paired {
        pairs.push((remaining_base[k], remaining_candidate[k]));
    }
    pairs
}

/// Records one `CallableMatch` per pair, tagged with `tier`.
fn record_matches(
    pairs: &[(usize, usize)],
    base_file: &FileCallables,
    candidate_file: &FileCallables,
    tier: MatchTier,
    matches: &mut Vec<CallableMatch>,
) {
    for &(base_index, candidate_index) in pairs {
        matches.push(CallableMatch {
            base: CallableRef {
                path: base_file.path.clone(),
                index: base_index,
            },
            candidate: CallableRef {
                path: candidate_file.path.clone(),
                index: candidate_index,
            },
            tier,
        });
    }
}

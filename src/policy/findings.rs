//! M3-2: diff-aware finding matching and V101 (`nsd-plan-final.md`
//! *Stable data model*, *Diagnostics*).

use crate::analysis::FileAnalysis;
use crate::config::PolicyConfig;
use crate::git::diff::Change;
use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::identity::matching::MatchOutput;
use crate::policy::diagnostics::FindingDiagnostic;

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

/// Matches `base` findings to `candidate` findings and raises V101 for
/// every unmatched candidate finding.
pub fn match_findings(
    base: &[FindingFile<'_>],
    candidate: &[FindingFile<'_>],
    matched: &MatchOutput,
    changes: &[Change],
    policy: &PolicyConfig,
) -> Result<FindingMatchOutput, GitError> {
    let _ = (base, candidate, matched, changes, policy);
    todo!("M3-2 finding matching")
}

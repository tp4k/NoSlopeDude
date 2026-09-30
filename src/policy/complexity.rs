//! M3-1: E101/E102 classification of matched callables, and G102 for a
//! callable-match ambiguity that could change a verdict.

use crate::config::PolicyConfig;
use crate::git::path::RepoPath;
use crate::identity::matching::MatchOutput;
use crate::model::Callable;
use crate::policy::diagnostics::PolicyDiagnostic;

/// One file's callable metrics from one snapshot, index-aligned with the
/// `FileCallables` the matcher saw for the same path.
#[derive(Debug, Clone)]
pub struct FileMetrics {
    pub path: RepoPath,
    pub callables: Vec<Callable>,
}

/// Classifies every candidate callable against `matched`.
pub fn classify(
    base: &[FileMetrics],
    candidate: &[FileMetrics],
    matched: &MatchOutput,
    policy: &PolicyConfig,
) -> Vec<PolicyDiagnostic> {
    let _ = (base, candidate, matched, policy);
    todo!("M3-1 classification")
}

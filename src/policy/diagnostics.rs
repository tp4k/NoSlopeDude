//! The complexity-policy diagnostic record (M3-1): code, candidate callable
//! and, when paired, its base callable. Carries no severity; the exit-code
//! mapping is M5-3's job.

use crate::git::path::RepoPath;
use crate::model::RuleId;

/// A candidate callable has `CC > 10` while its matched base callable is
/// absent or has `CC <= 10` (`nsd-plan-final.md` *Diagnostics*, `NSD-E101`).
pub const CODE_COMPLEXITY_ABOVE_THRESHOLD: &str = "NSD-E101";

/// Base and candidate both have `CC > 10`, and CC increased or SLOC grew past
/// `max(1, floor(base_sloc/10))` (`NSD-E102`).
pub const CODE_COMPLEXITY_INCREASED: &str = "NSD-E102";

/// A callable-match ambiguity that could change an E101/E102 verdict
/// (`NSD-G102`).
pub const CODE_MATCH_AMBIGUITY: &str = "NSD-G102";

/// A candidate core-rule finding with no matched base finding
/// (`NSD-V101`).
pub const CODE_UNMATCHED_FINDING: &str = "NSD-V101";

/// The matched base callable behind a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseCallable {
    pub path: RepoPath,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
}

/// One E101, E102 or G102 finding about a candidate callable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDiagnostic {
    pub code: &'static str,
    pub candidate_path: RepoPath,
    pub candidate_name: String,
    pub candidate_start_line: usize,
    pub candidate_end_line: usize,
    pub base: Option<BaseCallable>,
}

/// One V101 about an unmatched candidate finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingDiagnostic {
    pub code: &'static str,
    pub rule_id: RuleId,
    pub candidate_path: RepoPath,
    pub candidate_start_line: usize,
    pub candidate_end_line: usize,
}

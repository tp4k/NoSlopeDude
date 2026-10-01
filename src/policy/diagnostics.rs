//! The complexity-policy diagnostic record (M3-1): code, candidate callable
//! and, when paired, its base callable. Carries no severity; the exit-code
//! mapping is M5-3's job.

use crate::analysis::UnanalyzableReason;
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

/// A candidate finding is suppressed by a directive that is new, or whose
/// matched base finding was not suppressed (`NSD-S101`).
pub const CODE_NEW_SUPPRESSION: &str = "NSD-S101";

/// An invalid, unknown-rule or unused suppression directive (`NSD-S102`).
pub const CODE_INVALID_SUPPRESSION: &str = "NSD-S102";

/// One S101 or S102 about a candidate directive. `rule_id` is the directive's
/// target for an S101, and for an S102 only when the directive names a valid
/// rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionDiagnostic {
    pub code: &'static str,
    pub rule_id: Option<RuleId>,
    pub candidate_path: RepoPath,
    pub directive_line: usize,
}

/// A changed file's parse damage is new, unmappable, intersecting, or
/// worsened, or a changed callable was left unmeasured by it (`NSD-A101`).
pub const CODE_PARSE_DAMAGE: &str = "NSD-A101";

/// One A101 about a candidate file's damaged lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageDiagnostic {
    pub code: &'static str,
    pub candidate_path: RepoPath,
    pub candidate_start_line: usize,
    pub candidate_end_line: usize,
}

/// A required file could not be analyzed: too large, a non-UTF-8 path,
/// invalid encoding, or no parser (`NSD-A102`).
pub const CODE_ANALYSIS_UNAVAILABLE: &str = "NSD-A102";

/// One A102 about an included file and why it could not be analyzed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageDiagnostic {
    pub code: &'static str,
    pub path: RepoPath,
    pub reason: UnanalyzableReason,
}

/// A changed clone occurrence is new, or extended past the move threshold,
/// with a qualifying match elsewhere (`NSD-V102`).
pub const CODE_CLONE_REGRESSION: &str = "NSD-V102";

/// One V102 about a candidate clone occurrence. `base_lines` is the diff-mapped
/// base occurrence's `source_lines` for an extension and `None` for a new
/// occurrence; the `matched_*` fields name the first other member of the
/// occurrence's maximal group in canonical order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneDiagnostic {
    pub code: &'static str,
    pub candidate_path: RepoPath,
    pub candidate_start_line: usize,
    pub candidate_end_line: usize,
    pub base_lines: Option<usize>,
    pub added_lines: usize,
    pub matched_path: RepoPath,
    pub matched_start_line: usize,
    pub matched_end_line: usize,
}

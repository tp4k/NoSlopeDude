//! M3-3: source suppressions (`nsd-plan-final.md` *Stable data model*,
//! *Diagnostics*, A9).

use crate::config::PolicyConfig;
use crate::git::diff::Change;
use crate::git::GitError;
use crate::policy::diagnostics::{FindingDiagnostic, SuppressionDiagnostic};
use crate::policy::findings::{FindingFile, FindingMatchOutput};

/// The S101/S102 diagnostics, and the V101s left after suppression.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuppressionOutput {
    pub diagnostics: Vec<SuppressionDiagnostic>,
    pub findings: Vec<FindingDiagnostic>,
}

/// Applies suppressions to a matched base/candidate pair.
pub fn apply_suppressions(
    _base: &[FindingFile<'_>],
    _candidate: &[FindingFile<'_>],
    _matched: &FindingMatchOutput,
    _changes: &[Change],
    _policy: &PolicyConfig,
) -> Result<SuppressionOutput, GitError> {
    todo!()
}

/// One-snapshot semantics: every invalid and unused directive.
pub fn scan_suppressions(
    _files: &[FindingFile<'_>],
    _policy: &PolicyConfig,
) -> Vec<SuppressionDiagnostic> {
    todo!()
}

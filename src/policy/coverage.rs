//! M4-1: A102, required analysis coverage (`nsd-plan-final.md` *Required
//! clone coverage*).

use crate::analysis::UnanalyzableReason;
use crate::config::PolicyConfig;
use crate::git::discovery::IncludedEntry;
use crate::policy::diagnostics::CoverageDiagnostic;

/// One included entry with the caller's change status and analysis outcome.
#[derive(Debug, Clone)]
pub struct CoverageInput {
    pub entry: IncludedEntry,
    pub changed: bool,
    pub failure: Option<UnanalyzableReason>,
}

/// Raises A102 for every changed input that could not be analyzed, and for
/// every unchanged one when `V102` is `deny` or `warn`. Sorted by path bytes.
pub fn evaluate_coverage(
    inputs: &[CoverageInput],
    policy: &PolicyConfig,
) -> Vec<CoverageDiagnostic> {
    let _ = (inputs, policy);
    unimplemented!()
}

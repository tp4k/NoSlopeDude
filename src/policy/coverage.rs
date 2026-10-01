//! M4-1: A102, required analysis coverage (`nsd-plan-final.md` *Required
//! clone coverage*).

use crate::analysis::UnanalyzableReason;
use crate::config::{PolicyConfig, Severity};
use crate::git::discovery::IncludedEntry;
use crate::policy::diagnostics::{CoverageDiagnostic, CODE_ANALYSIS_UNAVAILABLE};

/// One included entry with the caller's change status and analysis outcome.
#[derive(Debug, Clone)]
pub struct CoverageInput {
    /// A discovery-included entry.
    pub entry: IncludedEntry,
    /// True when `entry.path` is the candidate-side path of a `git::diff::Change`: `Added`/`Modified`/`Typechange` `path`, `Renamed` `to`.
    pub changed: bool,
    /// `analyze_file(..).err()` when the bytes were read; an unreadable included blob must be reported as a failure, never `None`.
    pub failure: Option<UnanalyzableReason>,
}

/// The reason an input is unanalyzable: the entry's own flags first, then
/// the caller's analysis outcome.
fn reason_of(input: &CoverageInput) -> Option<UnanalyzableReason> {
    if input.entry.too_large {
        Some(UnanalyzableReason::TooLarge)
    } else if input.entry.non_utf8_path {
        Some(UnanalyzableReason::NonUtf8Path)
    } else {
        input.failure
    }
}

/// Raises A102 for every changed input that could not be analyzed, and for
/// every unchanged one when `V102` is `deny` or `warn`. Sorted by path bytes.
pub fn evaluate_coverage(
    inputs: &[CoverageInput],
    policy: &PolicyConfig,
) -> Vec<CoverageDiagnostic> {
    let unchanged_required = policy.nsd_v102 != Severity::Off;
    let mut diagnostics: Vec<CoverageDiagnostic> = inputs
        .iter()
        .filter(|input| input.changed || unchanged_required)
        .filter_map(|input| {
            let reason = reason_of(input)?;
            (reason != UnanalyzableReason::UnsupportedExtension).then(|| CoverageDiagnostic {
                code: CODE_ANALYSIS_UNAVAILABLE,
                path: input.entry.path.clone(),
                reason,
            })
        })
        .collect();
    diagnostics.sort_by(|a, b| a.path.cmp(&b.path));
    diagnostics
}

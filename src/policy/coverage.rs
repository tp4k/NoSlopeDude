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
    /// True when `entry.path` is the candidate-side path of a
    /// `git::diff::Change`: `Added`/`Modified`/`Typechange` `path`, `Renamed`
    /// `to`; or its base-side path, when the base copy of a changed file is
    /// too large or has a non-UTF-8 path.
    pub changed: bool,
    /// `analyze_file(..).err()` when the bytes were read. `reason_of` checks
    /// the entry's own `too_large` and `non_utf8_path` flags before this
    /// field, so the caller need not read such a blob to fill it. Any other
    /// included blob that should have been read but could not be is reported
    /// as a failure, never `None`.
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
/// every unchanged one when `V102` is `deny` or `warn`, except an unchanged
/// `UnsupportedExtension`, which raises nothing. Sorted by path bytes.
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
            let tolerated = !input.changed && reason == UnanalyzableReason::UnsupportedExtension;
            (!tolerated).then(|| CoverageDiagnostic {
                code: CODE_ANALYSIS_UNAVAILABLE,
                path: input.entry.path.clone(),
                reason,
            })
        })
        .collect();
    diagnostics.sort_by(|a, b| a.path.cmp(&b.path));
    diagnostics
}

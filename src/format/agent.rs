//! Agent listing of a check's violations.

use crate::check::CheckDiagnostic;
use crate::config::PolicyConfig;
use crate::policy::exit::contribution;

use super::capped_listing;

/// Prefix of the agent listing's omitted-count line.
const OMITTED_VIOLATIONS_LABEL: &str = "omitted violations:";

/// The violations among `diagnostics` (those with a non-zero exit
/// contribution under `policy` and the invocation's flag), in order: the
/// first `cap` of them, then `omitted violations: N` when `N` were dropped.
pub fn render_agent(
    diagnostics: &[CheckDiagnostic],
    policy: &PolicyConfig,
    allow_new_suppressions: bool,
    cap: u32,
) -> String {
    let violations: Vec<&CheckDiagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| contribution(diagnostic.code(), policy, allow_new_suppressions) != 0)
        .collect();
    capped_listing(violations.into_iter(), cap, OMITTED_VIOLATIONS_LABEL)
}

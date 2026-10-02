//! Exit-status aggregation over a complete diagnostic set (M5-3).

use crate::config::{PolicyConfig, Severity, CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG};
use crate::git::CODE_SNAPSHOT_UNAVAILABLE;
use crate::policy::diagnostics::{
    CODE_ANALYSIS_UNAVAILABLE, CODE_CLONE_REGRESSION, CODE_COMPLEXITY_ABOVE_THRESHOLD,
    CODE_COMPLEXITY_INCREASED, CODE_INVALID_SUPPRESSION, CODE_MATCH_AMBIGUITY,
    CODE_NEW_SUPPRESSION, CODE_PARSE_DAMAGE, CODE_UNMATCHED_FINDING,
};

const REGRESSION_BIT: u8 = 1;
const ERROR_BIT: u8 = 2;

/// Exit status for the diagnostic codes of one invocation: `0` pass, `1`
/// denied regressions, `2` analysis/config/snapshot errors, `3` both. An
/// unknown code fails closed as an error.
pub fn exit_status<'a>(
    codes: impl IntoIterator<Item = &'a str>,
    policy: &PolicyConfig,
    allow_new_suppressions: bool,
) -> u8 {
    codes.into_iter().fold(0, |status, code| {
        status | contribution(code, policy, allow_new_suppressions)
    })
}

fn contribution(code: &str, policy: &PolicyConfig, allow_new_suppressions: bool) -> u8 {
    match code {
        CODE_COMPLEXITY_ABOVE_THRESHOLD => denied(policy.nsd_e101),
        CODE_COMPLEXITY_INCREASED => denied(policy.nsd_e102),
        CODE_UNMATCHED_FINDING => denied(policy.nsd_v101),
        CODE_CLONE_REGRESSION => denied(policy.nsd_v102),
        CODE_INVALID_SUPPRESSION => denied(policy.nsd_s102),
        CODE_NEW_SUPPRESSION if allow_new_suppressions => 0,
        CODE_NEW_SUPPRESSION => REGRESSION_BIT,
        CODE_CONFIG_CHANGED => 0,
        CODE_PARSE_DAMAGE
        | CODE_ANALYSIS_UNAVAILABLE
        | CODE_SNAPSHOT_UNAVAILABLE
        | CODE_MATCH_AMBIGUITY
        | CODE_INVALID_CONFIG => ERROR_BIT,
        _ => ERROR_BIT,
    }
}

fn denied(severity: Severity) -> u8 {
    match severity {
        Severity::Deny => REGRESSION_BIT,
        Severity::Warn | Severity::Off => 0,
    }
}

//! Exit-status aggregation over a complete diagnostic set (M5-3).

use crate::config::PolicyConfig;

/// Exit status for the diagnostic codes of one invocation: `0` pass, `1`
/// denied regressions, `2` analysis/config/snapshot errors, `3` both.
pub fn exit_status<'a>(
    codes: impl IntoIterator<Item = &'a str>,
    policy: &PolicyConfig,
    allow_new_suppressions: bool,
) -> u8 {
    let _ = (codes.into_iter().count(), policy, allow_new_suppressions);
    unimplemented!()
}

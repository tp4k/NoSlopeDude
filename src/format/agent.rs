//! Agent listing of a check's violations.

use crate::check::CheckDiagnostic;
use crate::config::PolicyConfig;

/// The violations among `diagnostics`, capped at `cap`.
pub fn render_agent(
    _diagnostics: &[CheckDiagnostic],
    _policy: &PolicyConfig,
    _allow_new_suppressions: bool,
    _cap: u32,
) -> String {
    String::new()
}

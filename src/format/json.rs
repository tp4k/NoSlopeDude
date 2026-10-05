//! JSON rendering of a check's diagnostics (a subset of M6-1/M6-2).

use crate::check::CheckDiagnostic;

/// One compact JSON document and a newline.
pub fn render_json(diagnostics: &[CheckDiagnostic], exit_status: u8) -> String {
    let _ = (diagnostics, exit_status);
    unimplemented!("render_json")
}

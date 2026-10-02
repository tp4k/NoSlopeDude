//! Terminal listing of a check's diagnostics.

use crate::check::CheckDiagnostic;

/// One line per diagnostic, in the order given, each ended by a newline.
pub fn render_terminal(diagnostics: &[CheckDiagnostic]) -> String {
    let _ = diagnostics;
    todo!()
}

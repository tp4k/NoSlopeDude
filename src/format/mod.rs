//! Terminal listing of a check's diagnostics. Every line is escaped by
//! `escape_terminal`, because names, paths and messages come from a PR.

mod json;

use crate::check::CheckDiagnostic;
use crate::git::path::RepoPath;

pub use json::render_json;

/// The file a `CheckDiagnostic::Config` is about.
const CANDIDATE_CONFIG_FILE: &str = "nsd.yml";

/// One line per diagnostic, in the order given, each ended by a newline.
pub fn render_terminal(diagnostics: &[CheckDiagnostic]) -> String {
    let mut listing = String::new();
    for diagnostic in diagnostics {
        listing.push_str(&escape_terminal(&describe(diagnostic)));
        listing.push('\n');
    }
    listing
}

/// Replaces every control character (C0, DEL and C1) with `\u{..}`, so text
/// from a PR can neither move the cursor, start an escape sequence nor end
/// its own line.
pub fn escape_terminal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            escaped.push_str(&format!("\\u{{{:x}}}", u32::from(character)));
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn span(path: &RepoPath, start: usize, end: usize) -> String {
    format!("{}:{start}-{end}", path.render())
}

fn describe(diagnostic: &CheckDiagnostic) -> String {
    let code = diagnostic.code();
    match diagnostic {
        CheckDiagnostic::Complexity(found) => {
            let location = span(
                &found.candidate_path,
                found.candidate_start_line,
                found.candidate_end_line,
            );
            let mut line = format!("{code} {location} {}", found.candidate_name);
            if let Some(base) = &found.base {
                line.push_str(&format!(
                    " (base {} cc {} sloc {})",
                    span(&base.path, base.start_line, base.end_line),
                    base.cc,
                    base.sloc
                ));
            }
            line
        }
        CheckDiagnostic::Finding(found) => format!(
            "{code} {} {}",
            span(
                &found.candidate_path,
                found.candidate_start_line,
                found.candidate_end_line
            ),
            found.rule_id
        ),
        CheckDiagnostic::Suppression(found) => {
            let mut line = format!(
                "{code} {}:{}",
                found.candidate_path.render(),
                found.directive_line
            );
            if let Some(rule_id) = found.rule_id {
                line.push(' ');
                line.push_str(rule_id);
            }
            line
        }
        CheckDiagnostic::Damage(found) => format!(
            "{code} {}",
            span(
                &found.candidate_path,
                found.candidate_start_line,
                found.candidate_end_line
            )
        ),
        CheckDiagnostic::Coverage(found) => {
            format!("{code} {} {}", found.path.render(), found.reason.label())
        }
        CheckDiagnostic::Clone(found) => format!(
            "{code} {} (+{} lines) matches {}",
            span(
                &found.candidate_path,
                found.candidate_start_line,
                found.candidate_end_line
            ),
            found.added_lines,
            span(
                &found.matched_path,
                found.matched_start_line,
                found.matched_end_line
            )
        ),
        CheckDiagnostic::Config(_) => format!("{code} {CANDIDATE_CONFIG_FILE}"),
        CheckDiagnostic::Failure { message, .. } => format!("{code} {message}"),
    }
}

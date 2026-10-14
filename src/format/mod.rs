//! Terminal listing of a check's diagnostics. Every line is escaped by
//! `escape_terminal`, because names, paths and messages come from a PR.

mod agent;
mod json;

use crate::check::CheckDiagnostic;
use crate::git::path::RepoPath;

pub use agent::render_agent;
pub use json::{
    canonical_document, canonical_document_pretty, float, render_check_json, render_json,
    SCAN_RESULT_SCOPE, SCHEMA_VERSION,
};

/// Prefix of the terminal listing's omitted-count line.
const OMITTED_DIAGNOSTICS_LABEL: &str = "omitted diagnostics:";

/// The file a `CheckDiagnostic::Config` is about.
const CANDIDATE_CONFIG_FILE: &str = "nsd.yml";

/// One line per diagnostic, in the order given, each ended by a newline: the
/// first `cap` of them, then `omitted diagnostics: N` when `N` were dropped.
pub fn render_terminal(diagnostics: &[CheckDiagnostic], cap: u32) -> String {
    capped_listing(diagnostics.iter(), cap, OMITTED_DIAGNOSTICS_LABEL)
}

/// The listing of the first `cap` of `diagnostics`, escaped, followed by one
/// `<label> <omitted count>` line when any were dropped.
pub(crate) fn capped_listing<'a>(
    diagnostics: impl ExactSizeIterator<Item = &'a CheckDiagnostic>,
    cap: u32,
    omitted_label: &str,
) -> String {
    let total = diagnostics.len();
    let kept = usize::try_from(cap).map_or(total, |cap| cap.min(total));
    let mut listing = String::new();
    for diagnostic in diagnostics.take(kept) {
        listing.push_str(&escape_terminal(&describe(diagnostic)));
        listing.push('\n');
    }
    if total > kept {
        listing.push_str(&format!("{omitted_label} {}\n", total - kept));
    }
    listing
}

/// The Unicode Bidi_Control code points: ALM, LRM, RLM, the embeddings and
/// overrides, and the isolates. They reorder how a line reads (Trojan Source).
fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// Replaces every control character (C0, DEL and C1) and every bidi control
/// with `\u{..}`, so text from a PR can neither move the cursor, start an
/// escape sequence, end its own line nor reorder the line around it.
pub fn escape_terminal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() || is_bidi_control(character) {
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

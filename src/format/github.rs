//! GitHub Actions listing of a check's diagnostics: one `::notice` workflow
//! command per diagnostic. Paths, names and messages come from a PR, so every
//! value is encoded the way the runner's command parser decodes it, and
//! nothing a PR writes can end the line or add a property.

use crate::check::{CheckDetails, CheckDiagnostic};
use crate::config::{PolicyConfig, CODE_CONFIG_CHANGED};
use crate::policy::diagnostics::PolicyDiagnostic;
use crate::policy::exit::contribution;
use crate::report::published_name;

use super::{needs_escape, CANDIDATE_CONFIG_FILE, OMITTED_DIAGNOSTICS_LABEL};

/// The message of an A101, whose damaged spans only the JSON report details.
const DAMAGE_MESSAGE: &str = "damaged span; see the nsd JSON report";

/// The message of an S102 whose directive names no valid rule.
const UNNAMED_SUPPRESSION_MESSAGE: &str = "suppression directive";

/// One `::notice` line per diagnostic: the violations (a non-zero exit
/// contribution under `policy` and the invocation's flag) first, then the
/// rest, each group in input order. The first `cap` of those, then one plain
/// `omitted diagnostics: N of M` line when `N` of the `M` were dropped.
/// `details` supplies the candidate metrics of a paired E101/E102.
pub fn render_github(
    diagnostics: &[CheckDiagnostic],
    details: Option<&CheckDetails>,
    policy: &PolicyConfig,
    allow_new_suppressions: bool,
    cap: u32,
) -> String {
    let (violations, rest): (Vec<&CheckDiagnostic>, Vec<&CheckDiagnostic>) =
        diagnostics.iter().partition(|diagnostic| {
            contribution(diagnostic.code(), policy, allow_new_suppressions) != 0
        });
    let total = diagnostics.len();
    let kept = usize::try_from(cap).map_or(total, |cap| cap.min(total));
    let mut listing = String::new();
    for diagnostic in violations.into_iter().chain(rest).take(kept) {
        listing.push_str(&annotation(diagnostic, details));
        listing.push('\n');
    }
    if total > kept {
        listing.push_str(&format!(
            "{OMITTED_DIAGNOSTICS_LABEL} {} of {total}\n",
            total - kept
        ));
    }
    listing
}

/// Encodes a command's data (its message) as the runner decodes it: `%`
/// first, then CR and LF, and every other control or bidi character as
/// `\u{..}`, so the value stays on its own line and reads as written.
pub fn escape_github_data(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '%' => escaped.push_str("%25"),
            '\r' => escaped.push_str("%0D"),
            '\n' => escaped.push_str("%0A"),
            other if needs_escape(other) => {
                escaped.push_str(&format!("\\u{{{:x}}}", u32::from(other)));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// Encodes a command property's value: as data, then `:` and `,`, so the
/// value can neither end the property list nor start another property.
pub fn escape_github_property(text: &str) -> String {
    escape_github_data(text)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

/// Where an annotation points: a file and, within it, a line range.
struct Location {
    file: String,
    lines: Option<(usize, usize)>,
}

impl Location {
    fn span(file: String, start: usize, end: usize) -> Option<Location> {
        Some(Location {
            file,
            lines: Some((start, end)),
        })
    }

    fn file(file: String) -> Option<Location> {
        Some(Location { file, lines: None })
    }
}

fn annotation(diagnostic: &CheckDiagnostic, details: Option<&CheckDetails>) -> String {
    let (location, message) = locate_and_describe(diagnostic, details);
    let mut properties = vec![format!(
        "title={}",
        escape_github_property(&format!("nsd {}", diagnostic.code()))
    )];
    if let Some(location) = location {
        properties.push(format!("file={}", escape_github_property(&location.file)));
        if let Some((start, end)) = location.lines {
            properties.push(format!("line={start}"));
            properties.push(format!("endLine={end}"));
        }
    }
    format!(
        "::notice {}::{}",
        properties.join(","),
        escape_github_data(&message)
    )
}

fn locate_and_describe(
    diagnostic: &CheckDiagnostic,
    details: Option<&CheckDetails>,
) -> (Option<Location>, String) {
    match diagnostic {
        CheckDiagnostic::Complexity(found) => (
            Location::span(
                found.candidate_path.render(),
                found.candidate_start_line,
                found.candidate_end_line,
            ),
            joined(&[
                &published_name(&found.candidate_name, found.candidate_start_line),
                &complexity_change(found, details),
            ]),
        ),
        CheckDiagnostic::Finding(found) => (
            Location::span(
                found.candidate_path.render(),
                found.candidate_start_line,
                found.candidate_end_line,
            ),
            found.rule_id.to_string(),
        ),
        CheckDiagnostic::Suppression(found) => (
            Location::span(
                found.candidate_path.render(),
                found.directive_line,
                found.directive_line,
            ),
            found
                .rule_id
                .unwrap_or(UNNAMED_SUPPRESSION_MESSAGE)
                .to_string(),
        ),
        CheckDiagnostic::Damage(found) => (
            Location::span(
                found.candidate_path.render(),
                found.candidate_start_line,
                found.candidate_end_line,
            ),
            DAMAGE_MESSAGE.to_string(),
        ),
        CheckDiagnostic::Coverage(found) => (
            Location::file(found.path.render()),
            found.reason.label().to_string(),
        ),
        CheckDiagnostic::Clone(found) => (
            Location::span(
                found.candidate_path.render(),
                found.candidate_start_line,
                found.candidate_end_line,
            ),
            format!(
                "clone of {}:{}-{} (+{} lines)",
                found.matched_path.render(),
                found.matched_start_line,
                found.matched_end_line,
                found.added_lines
            ),
        ),
        CheckDiagnostic::Config(found) => (
            Location::file(CANDIDATE_CONFIG_FILE.to_string()),
            if found.code == CODE_CONFIG_CHANGED {
                format!("{CANDIDATE_CONFIG_FILE} changed")
            } else {
                format!("{CANDIDATE_CONFIG_FILE} is invalid")
            },
        ),
        CheckDiagnostic::Failure { message, .. } => (None, message.clone()),
    }
}

/// The base-to-candidate metrics of a paired callable, or the base's alone
/// when no changed candidate entity has the diagnostic's path, start line and
/// name; empty when there is no base.
fn complexity_change(found: &PolicyDiagnostic, details: Option<&CheckDetails>) -> String {
    let Some(base) = &found.base else {
        return String::new();
    };
    let path = found.candidate_path.render();
    let candidate = details.and_then(|details| {
        details.entities.iter().find(|entity| {
            entity.path == path
                && entity.start_line == found.candidate_start_line
                && entity.name == found.candidate_name
        })
    });
    match candidate {
        Some(entity) => format!(
            "CC {} -> {}, SLOC {} -> {}",
            base.cc, entity.cc, base.sloc, entity.sloc
        ),
        None => format!("base CC {}, SLOC {}", base.cc, base.sloc),
    }
}

/// The non-empty parts, joined by ` | `.
fn joined(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" | ")
}

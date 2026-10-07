//! JSON rendering of a check's diagnostics (a subset of M6-1/M6-2). Keys come
//! out sorted because `serde_json` is built without `preserve_order`.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::check::{CheckDetails, CheckDiagnostic, Entity, FamilySummary, FileCoverage, Gap};
use crate::git::path::RepoPath;
use crate::policy::diagnostics::BaseCallable;

use crate::report::published_name;

use super::CANDIDATE_CONFIG_FILE;

/// The `schema_version` of both scopes' canonical documents.
pub const SCHEMA_VERSION: u64 = 1;
const RESULT_SCOPE: &str = "check";
/// The `result_scope` of the scan document (`report.json`).
pub const SCAN_RESULT_SCOPE: &str = "scan";

/// The placeholder a hidden checkout path is replaced with in a message.
const HIDDEN_REPOSITORY: &str = "<repository>";
/// The placeholder a hidden config path is replaced with in a message.
const HIDDEN_CONFIG: &str = "<config>";

/// One compact JSON document and a newline: the four base keys only.
pub fn render_json(diagnostics: &[CheckDiagnostic], exit_status: u8) -> String {
    canonical_document(base_document(diagnostics, exit_status))
}

/// The canonical check document: the four base keys, and with `details` the
/// snapshots, fingerprints, skip counts, entities, summaries and coverage.
/// Failure messages hold none of `hidden_repository` or `hidden_config` (D8),
/// in the form given, joined to the working directory, or resolved.
pub fn render_check_json(
    diagnostics: &[CheckDiagnostic],
    exit_status: u8,
    details: Option<&CheckDetails>,
    hidden_repository: &[&Path],
    hidden_config: &[&Path],
) -> String {
    let hidden = Hidden::new(hidden_repository, hidden_config);
    let diagnostics: Vec<CheckDiagnostic> = diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            CheckDiagnostic::Failure { code, message } => CheckDiagnostic::Failure {
                code,
                message: hidden.scrub(message),
            },
            other => other.clone(),
        })
        .collect();
    let mut document = base_document(&diagnostics, exit_status);
    if let (Some(details), Value::Object(fields)) = (details, &mut document) {
        fields.extend(detail_fields(details));
    }
    canonical_document(document)
}

fn base_document(diagnostics: &[CheckDiagnostic], exit_status: u8) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "result_scope": RESULT_SCOPE,
        "exit_status": exit_status,
        "diagnostics": diagnostics.iter().map(entry).collect::<Vec<Value>>(),
    })
}

/// The canonical writer: `document` as one compact line and a newline.
/// Keys come out sorted (`serde_json` is built without `preserve_order`),
/// integers print as integers, and a float prints in its shortest
/// round-trip form with `-0.0` written as `0.0` (D6). A non-finite float
/// has no JSON spelling and is never built: `float` refuses it.
pub fn canonical_document(mut document: Value) -> String {
    normalize_floats(&mut document);
    format!("{document}\n")
}

/// The scan scope's canonical writer: the same normalized tree as
/// `canonical_document`, pretty-printed (stable layout, one key per line) and
/// ended by a newline.
pub fn canonical_document_pretty(mut document: Value) -> String {
    normalize_floats(&mut document);
    format!("{document:#}\n")
}

/// A float for a canonical document. Callers hold finite values only.
pub fn float(value: f64) -> Value {
    assert!(value.is_finite(), "a non-finite float has no JSON spelling");
    json!(if value == 0.0 { 0.0 } else { value })
}

fn normalize_floats(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if number.is_f64() && number.as_f64() == Some(0.0) {
                *number = serde_json::Number::from_f64(0.0).expect("0.0 is finite");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_floats),
        Value::Object(fields) => fields.values_mut().for_each(normalize_floats),
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

fn detail_fields(details: &CheckDetails) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert(
        "snapshots".to_string(),
        json!({
            "base": details.base_snapshot,
            "candidate": details.candidate_snapshot,
        }),
    );
    fields.insert(
        "fingerprints".to_string(),
        json!({
            "measurement": details.measurement_fingerprint,
            "configuration": details.configuration_fingerprint,
        }),
    );
    fields.insert("skipped".to_string(), json!(details.skipped));
    fields.insert(
        "entities".to_string(),
        Value::Array(details.entities.iter().map(entity).collect()),
    );
    let summaries = &details.summaries;
    fields.insert(
        "summaries".to_string(),
        json!({
            "scope": summaries.scope,
            "files": summaries.files,
            "overall": family(&summaries.overall),
            "java": family(&summaries.java),
            "js_ts": family(&summaries.js_ts),
        }),
    );
    fields.insert(
        "coverage".to_string(),
        Value::Array(details.coverage.iter().map(file_coverage).collect()),
    );
    fields
}

fn entity(entity: &Entity) -> Value {
    json!({
        "path": entity.path,
        "name": published_name(&entity.name, entity.start_line),
        "start_line": entity.start_line,
        "end_line": entity.end_line,
        "cc": entity.cc,
        "sloc": entity.sloc,
        "base": entity.base.as_ref().map(|base| json!({
            "path": base.path,
            "start_line": base.start_line,
            "end_line": base.end_line,
            "cc": base.cc,
            "sloc": base.sloc,
        })),
    })
}

fn family(summary: &FamilySummary) -> Value {
    json!({
        "erosion": float(summary.erosion),
        "verbosity": {
            "flagged_lines": summary.flagged_lines,
            "scanned_lines": summary.scanned_lines,
            "unanalyzed_lines": summary.unanalyzed_lines,
            "complete": summary.complete,
            "ratio": float(summary.ratio),
        },
    })
}

fn file_coverage(file: &FileCoverage) -> Value {
    json!({
        "path": file.path,
        "analyzed_lines": file.analyzed_lines,
        "unanalyzed_lines": file.unanalyzed_lines,
        "complete": file.complete,
        "gaps": file.gaps.iter().map(gap).collect::<Vec<Value>>(),
    })
}

fn line_range((start_line, end_line): (usize, usize)) -> Value {
    json!({ "start_line": start_line, "end_line": end_line })
}

fn gap(gap: &Gap) -> Value {
    json!({
        "base": gap.base.map(line_range),
        "candidate": line_range(gap.candidate),
        "tolerated": gap.tolerated,
    })
}

/// The path spellings a failure message must not carry, longest first.
struct Hidden {
    forms: Vec<(String, &'static str)>,
}

impl Hidden {
    fn new(repository: &[&Path], config: &[&Path]) -> Self {
        let mut forms: Vec<(String, &'static str)> = Vec::new();
        for (paths, label) in [(repository, HIDDEN_REPOSITORY), (config, HIDDEN_CONFIG)] {
            for path in paths {
                for form in spellings(path) {
                    forms.push((form, label));
                }
            }
        }
        forms.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        forms.dedup_by(|a, b| a.0 == b.0);
        Hidden { forms }
    }

    fn scrub(&self, message: &str) -> String {
        self.forms
            .iter()
            .fold(message.to_string(), |text, (form, label)| {
                text.replace(form.as_str(), label)
            })
    }
}

/// `path` as given and in its resolved form (the missing tail of a path that
/// does not exist is kept), without a trailing separator. A spelling of one
/// character or none, such as `/`, is dropped: replacing it would shred the
/// message.
fn spellings(path: &Path) -> Vec<String> {
    let mut found: Vec<PathBuf> = vec![path.to_path_buf()];
    if let Ok(resolved) = path.canonicalize() {
        found.push(resolved);
    } else if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(resolved) = parent.canonicalize() {
            found.push(resolved.join(name));
        }
    }
    found
        .iter()
        .filter_map(|form| form.to_str())
        .map(|text| text.trim_end_matches('/').to_string())
        .filter(|text| text.len() > 1)
        .collect()
}

fn entry(diagnostic: &CheckDiagnostic) -> Value {
    let mut fields = Map::new();
    fields.insert("code".to_string(), json!(diagnostic.code()));
    let more = match diagnostic {
        CheckDiagnostic::Complexity(found) => json!({
            "path": found.candidate_path.render(),
            "start_line": found.candidate_start_line,
            "end_line": found.candidate_end_line,
            "callable": found.candidate_name,
            "base": found.base.as_ref().map(base_callable),
        }),
        CheckDiagnostic::Finding(found) => json!({
            "path": found.candidate_path.render(),
            "start_line": found.candidate_start_line,
            "end_line": found.candidate_end_line,
            "rule_id": found.rule_id,
        }),
        CheckDiagnostic::Suppression(found) => json!({
            "path": found.candidate_path.render(),
            "directive_line": found.directive_line,
            "rule_id": found.rule_id,
        }),
        CheckDiagnostic::Damage(found) => json!({
            "path": found.candidate_path.render(),
            "start_line": found.candidate_start_line,
            "end_line": found.candidate_end_line,
        }),
        CheckDiagnostic::Coverage(found) => json!({
            "path": found.path.render(),
            "reason": found.reason.label(),
        }),
        CheckDiagnostic::Clone(found) => json!({
            "path": found.candidate_path.render(),
            "start_line": found.candidate_start_line,
            "end_line": found.candidate_end_line,
            "added_lines": found.added_lines,
            "matched": span(
                &found.matched_path,
                found.matched_start_line,
                found.matched_end_line,
            ),
        }),
        CheckDiagnostic::Config(_) => json!({ "path": CANDIDATE_CONFIG_FILE }),
        CheckDiagnostic::Failure { message, .. } => json!({ "message": message }),
    };
    if let Value::Object(extra) = more {
        fields.extend(extra);
    }
    Value::Object(fields)
}

fn span(path: &RepoPath, start_line: usize, end_line: usize) -> Value {
    json!({
        "path": path.render(),
        "start_line": start_line,
        "end_line": end_line,
    })
}

fn base_callable(base: &BaseCallable) -> Value {
    json!({
        "path": base.path.render(),
        "start_line": base.start_line,
        "end_line": base.end_line,
        "cc": base.cc,
        "sloc": base.sloc,
    })
}

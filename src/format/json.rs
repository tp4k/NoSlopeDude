//! JSON rendering of a check's diagnostics (a subset of M6-1/M6-2). Keys come
//! out sorted because `serde_json` is built without `preserve_order`.

use serde_json::{json, Map, Value};

use crate::check::CheckDiagnostic;
use crate::git::path::RepoPath;
use crate::policy::diagnostics::BaseCallable;

use super::CANDIDATE_CONFIG_FILE;

const SCHEMA_VERSION: u64 = 1;
const RESULT_SCOPE: &str = "check";

/// One compact JSON document and a newline.
pub fn render_json(diagnostics: &[CheckDiagnostic], exit_status: u8) -> String {
    let document = json!({
        "schema_version": SCHEMA_VERSION,
        "result_scope": RESULT_SCOPE,
        "exit_status": exit_status,
        "diagnostics": diagnostics.iter().map(entry).collect::<Vec<Value>>(),
    });
    format!("{document}\n")
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

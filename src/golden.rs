//! WS-4: the `java-fixture-01` golden digest (A1).
//!
//! `java-fixture-01`'s archived report lives outside this repository (it
//! carries private source excerpts and an absolute checkout path); this
//! module reads it as a bare `serde_json::Value` -- never `crate::model`'s
//! report structs, whose shape belongs to today's pre-IR engine and will
//! drift under M0b/M0c -- strips every `excerpt` and the absolute
//! `scan.target`, and reduces what remains to the seven sanitized
//! components the committed golden
//! (`tests/golden/java-fixture-01.digest.json`) holds. `tests/golden_digest.rs`
//! re-derives the same digest from the archive and checks it against the
//! committed file; see `docs/golden-digest.md`.

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::hashing::Digest;

/// The neutral label that replaces `scan.target`'s absolute checkout path.
pub(crate) const LABEL: &str = "java-fixture-01";

/// The fixture's language, verbatim from `nsd-plan-final.md`'s *Measured
/// starting state* row for `java-fixture-01`.
pub(crate) const LANGUAGE: &str = "Java, with a small JS/TS component";

/// Authorship is unknown for all three private fixtures; `AGENTS.md` says
/// outright not to infer it from complexity scores.
pub(crate) const AUTHORSHIP: &str = "unknown";

/// Mirrors `src/hashing.rs`'s private `HASH_VERSION` (`1` as of WS-3).
/// `hashing.rs` exposes no accessor for it, and this workstream's brief
/// forbids editing that module to add one (see the round-1 implementer
/// report's Refactor request). This digest's own `hash_version` metadata
/// field is therefore kept in sync by hand; a drift would first surface as
/// a loud failure in `hashing.rs`'s own `test_digest_is_stable_for_known_input`,
/// which pins a known digest value against the same constant.
const HASH_VERSION_MIRROR: u8 = 1;

/// The BLAKE3 family-prefix domain this digest hashes under, distinct from
/// `src/clones/mod.rs`'s per-language family prefixes, so a golden-digest
/// hash can never collide with a clone-run fingerprint even on identical
/// input bytes.
const BODY_FAMILY_PREFIX: &str = "golden-digest-body";

/// Recursively removes every `excerpt` key from `value`, at every nesting
/// depth, and returns how many were removed. A1's normalization names two
/// operations; this is the first, covering all three sites the archived
/// body carries it at (`findings[].location`, `duplicates[].locations[]`,
/// `top25[].location`) because it walks every object rather than a fixed
/// set of paths.
pub(crate) fn strip_excerpts(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut removed = usize::from(map.remove("excerpt").is_some());
            for nested in map.values_mut() {
                removed += strip_excerpts(nested);
            }
            removed
        }
        Value::Array(items) => items.iter_mut().map(strip_excerpts).sum(),
        _ => 0,
    }
}

/// Replaces `scan.target`'s absolute checkout path with `label`. A1's
/// second normalization.
fn relativize_target(report: &mut Value, label: &str) -> Result<()> {
    let scan = report
        .get_mut("scan")
        .and_then(Value::as_object_mut)
        .context("archived report is missing a `scan` object")?;
    scan.insert("target".to_string(), Value::String(label.to_string()));
    Ok(())
}

/// Runs both A1 normalizations on `report` in place and returns how many
/// `excerpt` occurrences were removed.
fn normalize(report: &mut Value, label: &str) -> Result<usize> {
    let removed = strip_excerpts(report);
    relativize_target(report, label)?;
    Ok(removed)
}

/// Canonical JSON bytes for the normalized report body. `serde_json::Map`
/// is backed by a `BTreeMap` unless this crate enables the `preserve_order`
/// feature (`Cargo.toml` does not), so `to_vec` already emits object keys
/// in sorted order; this function exists so that guarantee is named once
/// rather than assumed silently at the call site.
fn canonical_body_bytes(report: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(report).context("serializing the normalized report body")
}

/// The committed digest's `body_blake3`: WS-3's versioned BLAKE3 over the
/// canonical body bytes, algorithm-prefixed lowercase hex.
fn body_blake3(body: &[u8]) -> String {
    let mut digest = Digest::new(BODY_FAMILY_PREFIX);
    digest.push(body);
    format!("blake3:{:032x}", digest.finish())
}

/// `scan.revision.sha`, read back out of the (already normalized, but
/// revision-untouched) report -- the one absolute-checkout-independent
/// identity A1 keeps visible in the committed digest.
fn revision_sha(report: &Value) -> Result<String> {
    report
        .get("scan")
        .and_then(|scan| scan.get("revision"))
        .and_then(|revision| revision.get("sha"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("normalized report is missing scan.revision.sha")
}

/// `findings[].rule_id -> count`.
fn findings_by_rule_id(report: &Value) -> Result<Value> {
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .context("report is missing a `findings` array")?;
    let mut counts = std::collections::BTreeMap::new();
    for finding in findings {
        let rule_id = finding
            .get("rule_id")
            .and_then(Value::as_str)
            .context("a finding is missing `rule_id`")?;
        *counts.entry(rule_id.to_string()).or_insert(0u64) += 1;
    }
    Ok(json!(counts))
}

/// `{ group_count, total_redundant_lines }` over `duplicates`.
fn clone_summary(report: &Value) -> Result<Value> {
    let duplicates = report
        .get("duplicates")
        .and_then(Value::as_array)
        .context("report is missing a `duplicates` array")?;
    let group_count = duplicates.len() as u64;
    let mut total_redundant_lines = 0u64;
    for group in duplicates {
        total_redundant_lines += group
            .get("redundant_lines")
            .and_then(Value::as_u64)
            .context("a duplicate group is missing `redundant_lines`")?;
    }
    Ok(json!({
        "group_count": group_count,
        "total_redundant_lines": total_redundant_lines,
    }))
}

/// `skipped_files[].reason -> count`.
fn skips_by_reason(report: &Value) -> Result<Value> {
    let skipped = report
        .get("skipped_files")
        .and_then(Value::as_array)
        .context("report is missing a `skipped_files` array")?;
    let mut counts = std::collections::BTreeMap::new();
    for skip in skipped {
        let reason = skip
            .get("reason")
            .and_then(Value::as_str)
            .context("a skipped file is missing `reason`")?;
        *counts.entry(reason.to_string()).or_insert(0u64) += 1;
    }
    Ok(json!(counts))
}

/// `top25` as `[cc, sloc, mass]` triples, in the report's own order, name
/// and location dropped. Numbers are cloned straight out of the archived
/// `Value` rather than re-parsed through `f64`, so no digit of the
/// original representation can drift.
fn top25_triples(report: &Value) -> Result<Value> {
    let entries = report
        .get("top25")
        .and_then(Value::as_array)
        .context("report is missing a `top25` array")?;
    let triples: Result<Vec<Value>> = entries
        .iter()
        .map(|entry| {
            let cc = entry
                .get("cc")
                .cloned()
                .context("a top25 entry is missing `cc`")?;
            let sloc = entry
                .get("sloc")
                .cloned()
                .context("a top25 entry is missing `sloc`")?;
            let mass = entry
                .get("mass")
                .cloned()
                .context("a top25 entry is missing `mass`")?;
            Ok(Value::Array(vec![cc, sloc, mass]))
        })
        .collect();
    Ok(Value::Array(triples?))
}

/// Normalizes `report` per A1 and reduces it to the seven sanitized
/// components `tests/golden/java-fixture-01.digest.json` holds. Returns
/// the digest and how many `excerpt` occurrences the normalization
/// removed, so callers can assert that count against the archive's known
/// total.
pub(crate) fn build_digest(report: &mut Value) -> Result<(Value, usize)> {
    let removed = normalize(report, LABEL)?;
    let scores = report
        .get("scores")
        .cloned()
        .context("report is missing `scores`")?;
    let digest = json!({
        "label": LABEL,
        "language": LANGUAGE,
        "authorship": AUTHORSHIP,
        "revision_sha": revision_sha(report)?,
        "hash_version": HASH_VERSION_MIRROR,
        "scores": scores,
        "findings_by_rule_id": findings_by_rule_id(report)?,
        "clones": clone_summary(report)?,
        "skips_by_reason": skips_by_reason(report)?,
        "top25": top25_triples(report)?,
        "body_blake3": body_blake3(&canonical_body_bytes(report)?),
    });
    Ok((digest, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location_with_excerpt(relative_path: &str) -> serde_json::Value {
        json!({
            "relative_path": relative_path,
            "start_line": 1,
            "end_line": 2,
            "excerpt": "private source line",
            "link": format!("{relative_path}#L1-L2"),
            "is_remote_link": false,
        })
    }

    /// A synthetic report value carrying an `excerpt` at all three sites
    /// A1 normalizes -- `findings[].location`, `duplicates[].locations[]`,
    /// and `top25[].location` -- proving the strip removes all three (a
    /// fixture with excerpts only under `findings` and `duplicates` would
    /// pass while the real strip missed a quarter of the archived
    /// report's occurrences) and that hashing runs on the already
    /// normalized value, not the raw one.
    ///
    /// `build_digest` does not exist yet: this is WS-4's red commit.
    #[test]
    fn test_normalization_strips_excerpts_and_the_absolute_target() {
        let mut report = json!({
            "scan": {
                "target": "/Users/example/private-checkout",
                "revision": { "sha": "deadbeef", "dirty": false, "unavailable_reason": null },
            },
            "scores": { "overall": { "erosion": 0.5 } },
            "findings": [
                { "rule_id": "X", "language": "java", "location": location_with_excerpt("A.java") }
            ],
            "duplicates": [
                {
                    "language": "java",
                    "redundant_lines": 1,
                    "locations": [location_with_excerpt("B.java")],
                }
            ],
            "top25": [
                {
                    "name": "f",
                    "language": "java",
                    "cc": 1,
                    "sloc": 1,
                    "mass": 1.0,
                    "location": location_with_excerpt("C.java"),
                }
            ],
            "skipped_files": [],
        });

        let (digest, removed) = build_digest(&mut report).expect("build_digest");

        assert_eq!(removed, 3);
        assert_eq!(report["scan"]["target"], json!(LABEL));
        assert!(report["findings"][0]["location"].get("excerpt").is_none());
        assert!(report["duplicates"][0]["locations"][0]
            .get("excerpt")
            .is_none());
        assert!(report["top25"][0]["location"].get("excerpt").is_none());

        let body_blake3 = digest["body_blake3"]
            .as_str()
            .expect("body_blake3 is a string");
        assert!(body_blake3.starts_with("blake3:"));
        assert_eq!(body_blake3.len(), "blake3:".len() + 32);
    }
}

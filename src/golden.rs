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

/// Parses an archived report's raw JSON text into a `Value`, then repairs
/// the float leaves this digest reads (`scores.*.erosion`,
/// `scores.*.verbosity.ratio`, `top25[].mass`) against the same raw text.
///
/// This crate's pinned `serde_json` release (`Cargo.toml`, outside this
/// workstream's fenced scope) parses at least one real literal in the
/// `java-fixture-01` archive one ULP off its correctly-rounded value:
/// `scores.java.verbosity.ratio`'s `0.009966469683777625` round-trips back
/// out as `0.009966469683777623` through `serde_json::Value`, while Rust's
/// own `str::parse::<f64>` gets the same literal exactly right (see
/// `test_parse_report_corrects_a_known_serde_json_float_rounding_bug`
/// below, which reproduces this exact literal). `float_roundtrip` /
/// `arbitrary_precision` are `serde_json`'s own fixes for this, but both
/// require an edit to `Cargo.toml`, which this workstream may not touch;
/// re-deriving these specific leaves from the raw text is the compliant
/// workaround. Every other field this digest reads is an integer or string,
/// neither of which this bug touches.
pub(crate) fn parse_report(raw: &str) -> Result<Value> {
    let mut report: Value =
        serde_json::from_str(raw).context("parsing the archived report as JSON")?;
    correct_scores_floats(&mut report, raw)?;
    correct_top25_masses_keyed(&mut report, raw)?;
    Ok(report)
}

/// Parses a **committed digest**'s raw JSON text (the already-reduced
/// `tests/golden/java-fixture-01.digest.json` shape, not the archived
/// report) and applies the same raw-text float repair `parse_report` does.
/// A plain `serde_json::from_str` on the committed file hits the exact same
/// pinned-`serde_json` rounding bug `parse_report` works around -- the
/// digest file is JSON text like any other, and re-parsing it with the
/// buggy parser reintroduces the bug on read, even though the file on disk
/// already holds the correctly-rounded literal. `top25` here is already
/// reduced to bare `[cc, sloc, mass]` triples (no `mass` key survives
/// `build_digest`), so the mass leaves are found positionally rather than
/// by key -- see `correct_top25_masses_positional`.
pub(crate) fn parse_digest(raw: &str) -> Result<Value> {
    let mut digest: Value =
        serde_json::from_str(raw).context("parsing the committed digest as JSON")?;
    correct_scores_floats(&mut digest, raw)?;
    correct_top25_masses_positional(&mut digest, raw)?;
    Ok(digest)
}

/// How far a raw-text-recovered float may drift from what `serde_json`
/// itself parsed before it stops looking like the expected sub-ULP rounding
/// correction and starts looking like this function found the wrong number
/// entirely (a misaligned key search, for instance). Generous relative to a
/// true ULP (~1e-16 for values in this report's range), tight relative to a
/// real mismatch.
const MAX_RELATIVE_CORRECTION: f64 = 1e-9;

/// Re-parses `scores.overall/java/js_ts.{erosion, verbosity.ratio}` from
/// `raw`'s own text and overwrites `report`'s already-parsed values with
/// the result. Each language's object is located independently from a
/// fixed `scores`-object anchor, rather than chaining the search cursor
/// across languages: the archived report writes them `overall`, `java`,
/// `js_ts`, but the committed digest's `serde_json::Map` (a `BTreeMap`)
/// serializes object keys sorted, so there `java` comes first. Only the
/// two fields *within* one language's own object (`erosion` before
/// `verbosity.ratio`) are guaranteed to stay in that relative order.
fn correct_scores_floats(report: &mut Value, raw: &str) -> Result<()> {
    let scores_start = find_key_pos(raw, "scores", 0)?;
    for lang in ["overall", "java", "js_ts"] {
        let lang_pos = find_key_pos(raw, lang, scores_start)?;
        let (erosion, next) = number_for_key(raw, "erosion", lang_pos)?;
        let (ratio, _) = number_for_key(raw, "ratio", next)?;

        let language = report
            .get_mut("scores")
            .and_then(|scores| scores.get_mut(lang))
            .with_context(|| format!("normalized report is missing scores.{lang}"))?;
        let erosion_slot = language
            .get_mut("erosion")
            .with_context(|| format!("scores.{lang} is not an object with an `erosion` field"))?;
        overwrite_checked(erosion_slot, erosion)?;
        let ratio_slot = language
            .get_mut("verbosity")
            .and_then(|verbosity| verbosity.get_mut("ratio"))
            .with_context(|| {
                format!("scores.{lang}.verbosity is not an object with a `ratio` field")
            })?;
        overwrite_checked(ratio_slot, ratio)?;
    }
    Ok(())
}

/// Re-parses every `top25[].mass` from `raw`'s own text, in the array's own
/// order (JSON arrays -- unlike `serde_json`'s `BTreeMap`-backed objects --
/// preserve document order in both the raw text and the parsed `Value`, so
/// a plain left-to-right walk of each stays aligned with the other), and
/// overwrites `report`'s already-parsed values with the result.
fn correct_top25_masses_keyed(report: &mut Value, raw: &str) -> Result<()> {
    let entry_count = report
        .get("top25")
        .and_then(Value::as_array)
        .context("report is missing a `top25` array")?
        .len();

    let mut cursor = find_key_pos(raw, "top25", 0)?;
    let mut masses = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let (mass, next) = number_for_key(raw, "mass", cursor)?;
        masses.push(mass);
        cursor = next;
    }

    let entries = report
        .get_mut("top25")
        .and_then(Value::as_array_mut)
        .context("report is missing a `top25` array")?;
    for (entry, mass) in entries.iter_mut().zip(masses) {
        let slot = entry
            .get_mut("mass")
            .context("a top25 entry is not an object with a `mass` field")?;
        overwrite_checked(slot, mass)?;
    }
    Ok(())
}

/// Re-parses every `top25[i][2]` (the `mass` slot of each `[cc, sloc,
/// mass]` triple) from `raw`'s own text, in the array's own order, and
/// overwrites `digest`'s already-parsed values with the result. Unlike
/// [`correct_top25_masses_keyed`], the committed digest's triples carry no
/// key names, so each triple's three numbers are found positionally:
/// skip to the first number after `[`, skip to the second, skip to (and
/// keep) the third.
fn correct_top25_masses_positional(digest: &mut Value, raw: &str) -> Result<()> {
    let entry_count = digest
        .get("top25")
        .and_then(Value::as_array)
        .context("digest is missing a `top25` array")?
        .len();

    let mut cursor = find_key_pos(raw, "top25", 0)?;
    let mut masses = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        cursor = skip_to_number(raw, cursor);
        let (_cc, next) = number_after(raw, cursor)?;
        cursor = skip_to_number(raw, next);
        let (_sloc, next) = number_after(raw, cursor)?;
        cursor = skip_to_number(raw, next);
        let (mass, next) = number_after(raw, cursor)?;
        masses.push(mass);
        cursor = next;
    }

    let entries = digest
        .get_mut("top25")
        .and_then(Value::as_array_mut)
        .context("digest is missing a `top25` array")?;
    for (entry, mass) in entries.iter_mut().zip(masses) {
        let triple = entry
            .as_array_mut()
            .context("a top25 entry is not a [cc, sloc, mass] triple")?;
        let slot = triple
            .get_mut(2)
            .context("a top25 entry has fewer than 3 elements")?;
        overwrite_checked(slot, mass)?;
    }
    Ok(())
}

/// Skips forward from `from` past any character that cannot start a JSON
/// number (whitespace, `,`, `[`, `]`), stopping at the first digit or `-`.
fn skip_to_number(raw: &str, from: usize) -> usize {
    let bytes = raw.as_bytes();
    let mut i = from;
    while i < bytes.len() && bytes[i] != b'-' && !bytes[i].is_ascii_digit() {
        i += 1;
    }
    i
}

/// Overwrites `*slot` (a `serde_json`-parsed number) with `corrected`,
/// after checking the two agree to within [`MAX_RELATIVE_CORRECTION`] --
/// the guard that turns a raw-text search that landed on the wrong key into
/// a loud error instead of a silently wrong digest.
fn overwrite_checked(slot: &mut Value, corrected: f64) -> Result<()> {
    let original = slot
        .as_f64()
        .context("expected a float at the location being corrected")?;
    let tolerance = original.abs() * MAX_RELATIVE_CORRECTION;
    anyhow::ensure!(
        (corrected - original).abs() <= tolerance,
        "raw-text float recovery landed on an implausible value: \
         serde_json parsed {original}, raw text search found {corrected}"
    );
    *slot = json!(corrected);
    Ok(())
}

/// Finds the byte offset just past the literal `"key"` (its closing quote)
/// at or after `from`. Returning the position *after* the key, rather than
/// at its opening quote, matters for a key like `"top25"`, whose own text
/// contains digits: a scan for the next number that started inside the key
/// text itself would misread `25` from the key name as the first number of
/// the value that follows.
fn find_key_pos(raw: &str, key: &str, from: usize) -> Result<usize> {
    let pattern = format!("\"{key}\"");
    raw[from..]
        .find(pattern.as_str())
        .map(|offset| from + offset + pattern.len())
        .with_context(|| format!("could not find key `{key}` at or after byte {from}"))
}

/// Finds `"key"`'s value, assuming it is a bare JSON number, at or after
/// `from`, correctly rounded via `str::parse`. Returns the value and the
/// byte offset just past the number token, so callers can chain searches
/// forward without ever searching backward into text already consumed.
fn number_for_key(raw: &str, key: &str, from: usize) -> Result<(f64, usize)> {
    let key_pos = find_key_pos(raw, key, from)?;
    let colon_pos = raw[key_pos..]
        .find(':')
        .map(|offset| key_pos + offset + 1)
        .with_context(|| format!("no `:` after key `{key}` at byte {key_pos}"))?;
    number_after(raw, colon_pos)
}

/// Parses the JSON number token starting at or after `pos` (skipping
/// leading whitespace), and returns it plus the byte offset just past it.
fn number_after(raw: &str, pos: usize) -> Result<(f64, usize)> {
    let bytes = raw.as_bytes();
    let mut i = pos;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    let start = i;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    anyhow::ensure!(start < i, "expected a JSON number at byte {pos}");
    let token = &raw[start..i];
    let value = token
        .parse()
        .with_context(|| format!("parsing raw JSON number literal {token:?}"))?;
    Ok((value, i))
}

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

    /// L23 drift check (`HASH_VERSION_MIRROR`'s own doc comment names this
    /// test): computes `body_blake3(b"")`'s expected value independently,
    /// via a bare `blake3::Hasher` mirroring `hashing.rs`'s documented
    /// framing (version byte, then each slice as an 8-byte LE length plus
    /// its bytes, `BODY_FAMILY_PREFIX` first) under `HASH_VERSION_MIRROR`
    /// -- never through `hashing::Digest`, which is the code path
    /// `body_blake3` itself runs and so could never expose a drift between
    /// `HASH_VERSION_MIRROR` and the real `HASH_VERSION`. A future bump of
    /// `HASH_VERSION` without a matching bump of `HASH_VERSION_MIRROR`
    /// fails this test.
    #[test]
    fn test_body_blake3_of_empty_input_matches_hash_version_mirror() {
        let body: &[u8] = b"";
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[HASH_VERSION_MIRROR]);
        let prefix = BODY_FAMILY_PREFIX.as_bytes();
        hasher.update(&(prefix.len() as u64).to_le_bytes());
        hasher.update(prefix);
        hasher.update(&(body.len() as u64).to_le_bytes());
        hasher.update(body);
        let output_bytes = hasher.finalize();
        let mut leading = [0u8; 16];
        leading.copy_from_slice(&output_bytes.as_bytes()[..16]);
        let expected = format!("blake3:{:032x}", u128::from_le_bytes(leading));

        assert_eq!(body_blake3(body), expected);
    }

    /// L24: `overwrite_checked`'s callers used to reach their slot through
    /// `IndexMut` (`language["erosion"]`, `entry["mass"]`), which panics on
    /// a non-object container instead of returning the designed `anyhow`
    /// error. Covers both named call sites: a non-object `scores.java`, and
    /// a non-object `top25` entry.
    #[test]
    fn test_raw_text_recovery_rejects_a_non_object_container_without_panicking() {
        let non_object_scores_java = r#"{
            "scan": { "target": "/x", "revision": { "sha": "deadbeef", "dirty": false, "unavailable_reason": null } },
            "scores": {
                "overall": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "java": 5,
                "js_ts": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } }
            },
            "findings": [],
            "duplicates": [],
            "top25": [],
            "skipped_files": []
        }"#;
        let err = parse_report(non_object_scores_java).expect_err(
            "a non-object scores.java container must be rejected with an error, not a panic",
        );
        assert!(err.to_string().contains("java"), "error was: {err}");

        let non_object_top25_entry = r#"{
            "scan": { "target": "/x", "revision": { "sha": "deadbeef", "dirty": false, "unavailable_reason": null } },
            "scores": {
                "overall": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "java": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "js_ts": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } }
            },
            "findings": [],
            "duplicates": [],
            "top25": [42],
            "decoy": { "mass": 9.0 },
            "skipped_files": []
        }"#;
        let err = parse_report(non_object_top25_entry)
            .expect_err("a non-object top25 entry must be rejected with an error, not a panic");
        assert!(err.to_string().contains("mass"), "error was: {err}");
    }

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
                "target": "/example/checkout",
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

    /// Reproduces the exact literal that exposed this crate's pinned
    /// `serde_json` release's float-parsing bug in the real
    /// `java-fixture-01` archive (`scores.java.verbosity.ratio`): parsed
    /// via plain `serde_json::from_str::<Value>`, `0.009966469683777625`
    /// round-trips out as `0.009966469683777623`, one ULP low. Proves
    /// `parse_report` recovers the correctly-rounded value instead.
    #[test]
    fn test_parse_report_corrects_a_known_serde_json_float_rounding_bug() {
        let raw = r#"{
            "scan": { "target": "/x", "revision": { "sha": "deadbeef", "dirty": false, "unavailable_reason": null } },
            "scores": {
                "overall": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "java": {
                    "erosion": 0.1,
                    "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.009966469683777625 }
                },
                "js_ts": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } }
            },
            "findings": [],
            "duplicates": [],
            "top25": [],
            "skipped_files": []
        }"#;

        let naive: Value = serde_json::from_str(raw).expect("naive parse");
        let naive_ratio = naive["scores"]["java"]["verbosity"]["ratio"]
            .as_f64()
            .expect("naive ratio");
        assert_ne!(
            naive_ratio.to_bits(),
            "0.009966469683777625".parse::<f64>().unwrap().to_bits(),
            "this test's premise (plain serde_json mis-parses this literal) no longer holds; \
             `parse_report`'s raw-text correction may now be unnecessary"
        );

        let corrected = parse_report(raw).expect("parse_report");
        let corrected_ratio = corrected["scores"]["java"]["verbosity"]["ratio"]
            .as_f64()
            .expect("corrected ratio");
        assert_eq!(
            corrected_ratio.to_bits(),
            "0.009966469683777625".parse::<f64>().unwrap().to_bits()
        );
    }

    /// The same rounding bug reproduces when reading back a **committed
    /// digest**, whose `top25` triples carry no `mass` key to search for
    /// (`build_digest` already reduced them to bare `[cc, sloc, mass]`
    /// arrays) -- `parse_digest` must recover the third slot positionally.
    #[test]
    fn test_parse_digest_corrects_a_known_serde_json_float_rounding_bug_in_top25() {
        let raw = r#"{
            "scores": {
                "overall": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "java": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } },
                "js_ts": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } }
            },
            "top25": [
                [19, 125, 0.009966469683777625]
            ]
        }"#;

        let naive: Value = serde_json::from_str(raw).expect("naive parse");
        let naive_mass = naive["top25"][0][2].as_f64().expect("naive mass");
        assert_ne!(
            naive_mass.to_bits(),
            "0.009966469683777625".parse::<f64>().unwrap().to_bits(),
            "this test's premise (plain serde_json mis-parses this literal) no longer holds"
        );

        let corrected = parse_digest(raw).expect("parse_digest");
        let corrected_mass = corrected["top25"][0][2].as_f64().expect("corrected mass");
        assert_eq!(
            corrected_mass.to_bits(),
            "0.009966469683777625".parse::<f64>().unwrap().to_bits()
        );
    }

    /// `correct_scores_floats`'s raw-text search assumes `erosion` precedes
    /// `verbosity.ratio` *within* one language's own object (see its own
    /// doc comment) -- once it has found `erosion`, it searches forward for
    /// `ratio` starting just past it, never backward. This report violates
    /// that assumption for `scores.overall` (`verbosity` -- and so its own
    /// `ratio` -- is written *before* `erosion`), so the forward search for
    /// `overall`'s `ratio` skips past its own, genuinely-present value and
    /// lands on `scores.java`'s `ratio` instead. `overall`'s real ratio
    /// (`0.01`) and `java`'s (`0.99`) are grossly different so the
    /// misaligned read is guaranteed to fail
    /// [`MAX_RELATIVE_CORRECTION`]'s plausibility guard, proving that guard
    /// actually rejects a misaligned search rather than only ever seeing
    /// values close enough to pass. Widening `MAX_RELATIVE_CORRECTION` to
    /// something like `1e9`, or deleting the `ensure!` in
    /// `overwrite_checked`, makes this test fail.
    #[test]
    fn test_raw_text_recovery_rejects_a_misaligned_search() {
        let raw = r#"{
            "scan": { "target": "/x", "revision": { "sha": "deadbeef", "dirty": false, "unavailable_reason": null } },
            "scores": {
                "overall": {
                    "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.01 },
                    "erosion": 0.1
                },
                "java": {
                    "erosion": 0.9,
                    "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.99 }
                },
                "js_ts": { "erosion": 0.1, "verbosity": { "flagged_lines": 1, "scanned_lines": 1, "ratio": 0.1 } }
            },
            "findings": [],
            "duplicates": [],
            "top25": [],
            "skipped_files": []
        }"#;

        let err = parse_report(raw).expect_err(
            "a misaligned raw-text search over scores.overall's reordered fields \
             must be rejected, not silently committed as a wrong number",
        );
        assert!(err.to_string().contains("implausible"), "error was: {err}");
    }
}

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

use serde_json::json;

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

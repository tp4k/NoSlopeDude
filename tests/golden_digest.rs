//! WS-4: the `java-fixture-01` golden digest (A1).
//!
//! `src/golden.rs` is `#[cfg(test)]`-gated and `pub(crate)`, so it is not
//! reachable through the compiled `nsd` library from an external
//! integration-test crate. This file includes it (and `src/hashing.rs`,
//! which it depends on for `body_blake3`) directly via `#[path]` instead of
//! reimplementing the digest computation a second time -- the same source
//! is compiled twice, once inside the library crate and once here, rather
//! than being duplicated by hand.
//!
//! The archived `java-fixture-01` report never enters this repository; its
//! path is read only from `NSD_ARCHIVED_REPORT` at test-invocation time.
//! See `docs/golden-digest.md` for how to resolve and pass it.

#[path = "../src/golden.rs"]
mod golden;
#[path = "../src/hashing.rs"]
mod hashing;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use nsd::model::ScanSettings;
use nsd::pipeline;

/// The env var that carries the archived report's path at invocation time
/// only; it is never written into a repository file.
const ARCHIVED_REPORT_ENV_VAR: &str = "NSD_ARCHIVED_REPORT";

/// Opt-in env var that promotes the pending arm below from a silent `ok` to
/// a panic. Unset (the default for every local/dev run), a missing archive
/// still reads as a plain Cargo pass -- this var exists so a CI job that
/// *does* have the archive can demand one and fail loudly if it is
/// absent, without changing the test's default, archive-optional behavior.
const REQUIRE_ARCHIVE_VERIFIED_ENV_VAR: &str = "NSD_REQUIRE_ARCHIVE_VERIFIED";

/// How many `excerpt` occurrences the real archived `java-fixture-01`
/// report carries at the three sites A1 normalizes: `findings[].location`
/// (340) + `duplicates[].locations[]` (172) + `top25[].location` (25).
const EXPECTED_EXCERPT_COUNT: usize = 537;

/// `top25` is defined as exactly 25 entries; this is a property of the
/// digest's own format, not a measurement of this particular fixture.
const TOP25_TRIPLE_COUNT: usize = 25;

/// Cross-checked against `nsd-plan-final.md:40` (*Measured starting
/// state*). The js_ts erosion, the finding/rule-ID counts, the clone-group
/// count, and the top25 triples have no independent source outside the
/// archived report itself and are verified only by the equality check
/// against the committed golden below -- typing them in here would be the
/// same unverifiable hand-entry the golden file itself must avoid.
const EXPECTED_OVERALL_EROSION: f64 = 0.13313797553867948;
const EXPECTED_JAVA_EROSION: f64 = 0.13294913151043064;

/// Cross-checked against `nsd-plan-final.md:70` (*Corrections applied*,
/// row 7): 854 skips split `test: 853` / `dependency_or_build_output: 1`.
const EXPECTED_SKIPS_TEST: u64 = 853;
const EXPECTED_SKIPS_DEPENDENCY_OR_BUILD_OUTPUT: u64 = 1;

/// Opt-in env var, mirroring `tests/neutrality.rs::NEUTRALITY_CAPTURE_ENV_VAR`,
/// that promotes `test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01`
/// from a comparison into an implementer-only capture: instead of asserting
/// the live digest against the committed `nsd-v1` file, it writes the live
/// digest there. Never set in a verifier or CI run -- capturing overwrites a
/// repository file.
const GOLDEN_CAPTURE_ENV_VAR: &str = "NSD_GOLDEN_CAPTURE";

fn committed_digest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/java-fixture-01.digest.json")
}

/// The post-swap digest (D15): captured from a live scan once orchard, the
/// `nsd-v1` freeze, and the `end_line`/`-0.0` fixes have all landed, so it
/// matches what `nsd-v1` actually emits. `committed_digest_path` above must
/// keep reading the M0b file -- the archive-backed check that reproduces the
/// pre-swap golden is never repointed at this one.
fn nsd_v1_digest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/java-fixture-01.nsd-v1.digest.json")
}

/// Reads and parses a committed digest file at `path` -- either the M0b
/// digest or the `nsd-v1` digest, both the same seven-normalized, A1 shape.
fn read_committed_digest(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading the committed golden digest at {path:?}"))?;
    golden::parse_digest(&raw).context("parsing the committed golden digest")
}

/// Where the archived report is, or an explicit statement that this run's
/// leg is pending because the private fixture was not supplied -- so a
/// missing private fixture can never silently read as a validated pass
/// (`AGENTS.md` -> *Verification*).
enum ArchiveGate {
    Resolved(PathBuf),
    Pending,
}

fn archive_gate() -> ArchiveGate {
    classify_archive_gate(std::env::var_os(ARCHIVED_REPORT_ENV_VAR))
}

/// Pure classification of a possibly-absent `NSD_ARCHIVED_REPORT` value,
/// split out from `archive_gate`'s `env::var_os` call so `None` (the var is
/// unset) and `Some("")` (the var is set but empty) can each be asserted
/// directly, rather than only observed indirectly through whatever the test
/// process's own environment happens to carry when it runs.
fn classify_archive_gate(raw: Option<OsString>) -> ArchiveGate {
    match raw {
        Some(path) if !path.is_empty() => ArchiveGate::Resolved(PathBuf::from(path)),
        _ => ArchiveGate::Pending,
    }
}

/// The notice printed on every pending arm below -- extracted so the call
/// sites cannot drift apart and so a test can assert its content once.
fn pending_notice() -> String {
    format!(
        "PENDING: {ARCHIVED_REPORT_ENV_VAR} is unset -- java-fixture-01's \
         reproducibility gate is pending, not passing, this run"
    )
}

/// Pure classification of a possibly-absent `NSD_REQUIRE_ARCHIVE_VERIFIED`
/// value, split out the same way `classify_archive_gate` is: so `None` and
/// `Some("")` (unset, and set-but-empty) can each be asserted directly as
/// "not required", rather than only observed through the process's own
/// environment.
fn classify_verification_requirement(raw: Option<OsString>) -> bool {
    matches!(raw, Some(value) if !value.is_empty())
}

fn verification_required() -> bool {
    classify_verification_requirement(std::env::var_os(REQUIRE_ARCHIVE_VERIFIED_ENV_VAR))
}

/// The message a required-but-pending run panics with -- extracted so a
/// test can assert its content once, same as `pending_notice` above.
fn required_but_pending_message() -> String {
    format!(
        "{REQUIRE_ARCHIVE_VERIFIED_ENV_VAR} demands a verified run, but \
         {ARCHIVED_REPORT_ENV_VAR} is unset -- supply the archive or unset \
         {REQUIRE_ARCHIVE_VERIFIED_ENV_VAR}"
    )
}

/// Recomputes the digest from the archived report and asserts it against
/// the committed golden, proving the golden was derived and is
/// re-derivable rather than typed in by hand.
///
/// A missing archive never invents a pass here: the pending arm asserts
/// nothing. By default that pending run still reports as Cargo's plain
/// `ok`, same as a verified one -- `NSD_REQUIRE_ARCHIVE_VERIFIED` is the
/// opt-in a CI job with the archive can set to turn that silence into a
/// panic instead.
#[test]
fn test_committed_digest_matches_the_archived_report() -> Result<()> {
    let path = match archive_gate() {
        ArchiveGate::Resolved(path) => path,
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
            if verification_required() {
                panic!("{}", required_but_pending_message());
            }
            return Ok(());
        }
    };

    let raw = fs::read_to_string(&path).context("reading the archived report")?;
    let mut report = golden::parse_report(&raw).context("parsing the archived report")?;

    let (recomputed, removed) =
        golden::build_digest(&mut report).context("building the golden digest")?;

    assert_eq!(
        removed, EXPECTED_EXCERPT_COUNT,
        "excerpt strip must remove all three sites' occurrences, not a subset"
    );

    let overall_erosion = recomputed["scores"]["overall"]["erosion"]
        .as_f64()
        .context("recomputed digest is missing scores.overall.erosion")?;
    assert_eq!(overall_erosion, EXPECTED_OVERALL_EROSION);
    let java_erosion = recomputed["scores"]["java"]["erosion"]
        .as_f64()
        .context("recomputed digest is missing scores.java.erosion")?;
    assert_eq!(java_erosion, EXPECTED_JAVA_EROSION);

    let skips = recomputed["skips_by_reason"]
        .as_object()
        .context("recomputed digest is missing skips_by_reason")?;
    assert_eq!(
        skips.get("test").and_then(Value::as_u64),
        Some(EXPECTED_SKIPS_TEST)
    );
    assert_eq!(
        skips
            .get("dependency_or_build_output")
            .and_then(Value::as_u64),
        Some(EXPECTED_SKIPS_DEPENDENCY_OR_BUILD_OUTPUT)
    );

    let top25 = recomputed["top25"]
        .as_array()
        .context("recomputed digest is missing top25")?;
    assert_eq!(top25.len(), TOP25_TRIPLE_COUNT);

    let committed = read_committed_digest(&committed_digest_path())?;
    assert_eq!(
        recomputed, committed,
        "the committed golden must equal what build_digest re-derives from the archive"
    );

    Ok(())
}

/// Proves the pending path is reachable and prints its notice rather than
/// silently substituting a pass, independent of whether this particular
/// invocation happens to carry the archive-backed leg too.
#[test]
fn test_gate_is_reported_pending_when_the_archive_is_absent() {
    match archive_gate() {
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
        }
        ArchiveGate::Resolved(_) => {
            // The archive-backed leg is running in this invocation; the
            // pending branch above is exercised by this same test in the
            // ordinary (archive-absent) developer/CI run instead.
        }
    }
}

/// `classify_archive_gate` is the pure decision `archive_gate` delegates to;
/// tested directly (not through `env::var_os`, which only the process's own
/// environment can drive) so the unset case, the set-but-empty case, and the
/// set-and-non-empty case are each pinned rather than only exercised
/// incidentally by whichever of the three the test process happens to run
/// under.
#[test]
fn test_classify_archive_gate() {
    assert!(matches!(classify_archive_gate(None), ArchiveGate::Pending));
    assert!(matches!(
        classify_archive_gate(Some(OsString::new())),
        ArchiveGate::Pending
    ));
    match classify_archive_gate(Some(OsString::from("/x"))) {
        ArchiveGate::Resolved(path) => assert_eq!(path, PathBuf::from("/x")),
        ArchiveGate::Pending => panic!("a non-empty path must resolve, not read as pending"),
    }
}

/// The pending notice both pending arms above print must actually say the
/// run is pending and name the env var a developer needs to set -- proving
/// the two call sites cannot silently drift into printing nothing or an
/// unrelated message.
#[test]
fn test_pending_notice_names_the_env_var() {
    let notice = pending_notice();
    assert!(notice.contains("PENDING"), "notice was: {notice:?}");
    assert!(
        notice.contains(ARCHIVED_REPORT_ENV_VAR),
        "notice was: {notice:?}"
    );
}

/// `classify_verification_requirement` is the pure decision
/// `verification_required` delegates to; tested directly for the same
/// reason `classify_archive_gate` is -- the unset and set-but-empty cases
/// must read as "not required", not just happen to.
#[test]
fn test_classify_verification_requirement_needs_a_non_empty_value() {
    assert!(!classify_verification_requirement(None));
    assert!(!classify_verification_requirement(Some(OsString::new())));
    assert!(classify_verification_requirement(Some(OsString::from("1"))));
}

/// The panic message a required-but-pending run raises must name both env
/// vars, so a developer or CI log immediately says what is missing and
/// what to unset to get a plain pending run back.
#[test]
fn test_required_but_pending_message_names_both_env_vars() {
    let message = required_but_pending_message();
    assert!(
        message.contains(REQUIRE_ARCHIVE_VERIFIED_ENV_VAR),
        "message was: {message:?}"
    );
    assert!(
        message.contains(ARCHIVED_REPORT_ENV_VAR),
        "message was: {message:?}"
    );
}

/// Which `scores.*.verbosity` key set a committed digest is pinned to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VerbosityShape {
    /// The M0b file, frozen as history.
    M0b,
    /// The `nsd-v1` file since M6-2b: M1-8's coverage keys added.
    WithCoverage,
}

impl VerbosityShape {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::M0b => &["flagged_lines", "ratio", "scanned_lines"],
            Self::WithCoverage => &[
                "complete",
                "flagged_lines",
                "ratio",
                "scanned_lines",
                "unanalyzed_lines",
            ],
        }
    }
}

/// Runs unconditionally (no archive needed): a committed digest file --
/// either the M0b digest or the `nsd-v1` digest, both the same A1 shape --
/// must carry none of the archived report's private shape. Shared by
/// `test_committed_digest_carries_no_paths_names_or_excerpts` and
/// `test_nsd_v1_digest_carries_no_paths_names_or_excerpts` so the check
/// cannot silently drift between the two files it is run against.
///
/// `verbosity_shape` is the one thing the two files legitimately disagree
/// on: the M0b file keeps its three-key verbosity, while the `nsd-v1` file
/// also carries M1-8's `complete` (a boolean, the only non-numeric leaf the
/// digest may hold) and `unanalyzed_lines`.
fn assert_digest_carries_no_paths_names_or_excerpts(
    committed: &Value,
    verbosity_shape: VerbosityShape,
) -> Result<()> {
    const FORBIDDEN_KEYS: [&str; 6] = [
        "excerpt",
        "link",
        "name",
        "relative_path",
        "target",
        "detail",
    ];

    /// A path/link-shaped string can appear anywhere -- not just leading
    /// with `/` -- so this is deliberately wider than the pre-existing
    /// `starts_with('/')` check it sits alongside, additively.
    fn contains_path_shape_chars(text: &str) -> bool {
        text.contains('/') || text.contains('\\') || text.contains('~')
    }

    /// The one field this digest carries whose value is a fixed,
    /// hand-authored free-text label rather than anything read out of the
    /// archive: `src/golden.rs`'s `LANGUAGE` constant, verbatim from
    /// `nsd-plan-final.md`'s *Measured starting state* row. The `/` in
    /// `"JS/TS"` is not a path. Every archive-derived leaf this digest holds
    /// is a number (`scores`, `clones`, `findings_by_rule_id`,
    /// `skips_by_reason`, `top25` -- see the leaf-number check below), so
    /// this exact string is the only value that needs a named exemption
    /// from the widened path-shape scan; the exemption is by exact value,
    /// not by key, so a path-shaped string substituted for a genuine
    /// `language` value would still be caught. `label`, `authorship`,
    /// `revision_sha`, and `body_blake3` are separately pinned to their
    /// exact expected values right below, which is a strictly stronger
    /// check than a path-shape scan for each of them.
    const EXPECTED_LANGUAGE: &str = "Java, with a small JS/TS component";

    fn walk(value: &Value, forbidden_keys_found: &mut BTreeMap<String, ()>) {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    if FORBIDDEN_KEYS.contains(&key.as_str()) {
                        forbidden_keys_found.insert(key.clone(), ());
                    }
                    assert!(
                        !contains_path_shape_chars(key),
                        "committed digest carries a path-shaped object key: {key:?}"
                    );
                    walk(nested, forbidden_keys_found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, forbidden_keys_found);
                }
            }
            Value::String(text) => {
                assert!(
                    !text.starts_with('/'),
                    "committed digest carries an absolute-path-shaped string value: {text:?}"
                );
                if text != EXPECTED_LANGUAGE {
                    assert!(
                        !contains_path_shape_chars(text),
                        "committed digest carries a path-shaped string value: {text:?}"
                    );
                }
            }
            _ => {}
        }
    }

    let mut forbidden_keys_found = BTreeMap::new();
    walk(committed, &mut forbidden_keys_found);
    assert!(
        forbidden_keys_found.is_empty(),
        "committed digest carries forbidden keys: {:?}",
        forbidden_keys_found.keys().collect::<Vec<_>>()
    );

    const EXPECTED_TOP_LEVEL_KEYS: [&str; 11] = [
        "authorship",
        "body_blake3",
        "clones",
        "findings_by_rule_id",
        "hash_version",
        "label",
        "language",
        "revision_sha",
        "scores",
        "skips_by_reason",
        "top25",
    ];
    let top_level = committed
        .as_object()
        .context("committed digest is not a JSON object")?;
    let top_level_keys: BTreeSet<&str> = top_level.keys().map(String::as_str).collect();
    let expected_top_level_keys: BTreeSet<&str> = EXPECTED_TOP_LEVEL_KEYS.into_iter().collect();
    assert_eq!(
        top_level_keys, expected_top_level_keys,
        "committed digest's top-level key set must be exactly the eleven A1 fields"
    );

    assert_eq!(
        committed["label"],
        Value::String("java-fixture-01".to_string())
    );
    assert_eq!(
        committed["authorship"],
        Value::String("unknown".to_string())
    );
    assert_eq!(
        committed["revision_sha"],
        Value::String("c6671504394b7c862dac032bc7c7364ad3af8b7f".to_string())
    );
    assert_eq!(
        committed["language"],
        Value::String(EXPECTED_LANGUAGE.to_string())
    );

    let body_blake3 = committed["body_blake3"]
        .as_str()
        .context("body_blake3 is not a string")?;
    let hex = body_blake3
        .strip_prefix("blake3:")
        .with_context(|| format!("body_blake3 is missing the blake3: prefix: {body_blake3:?}"))?;
    assert_eq!(
        hex.len(),
        32,
        "body_blake3's hex portion must be 32 characters: {hex:?}"
    );
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "body_blake3's hex portion must be lowercase hex: {hex:?}"
    );

    const EXPECTED_LANGUAGES: [&str; 3] = ["overall", "java", "js_ts"];
    const EXPECTED_LANGUAGE_KEYS: [&str; 2] = ["erosion", "verbosity"];
    let expected_verbosity_keys: BTreeSet<&str> = verbosity_shape.keys().iter().copied().collect();
    let scores = committed["scores"]
        .as_object()
        .context("scores is not an object")?;
    let score_langs: BTreeSet<&str> = scores.keys().map(String::as_str).collect();
    let expected_langs: BTreeSet<&str> = EXPECTED_LANGUAGES.into_iter().collect();
    assert_eq!(
        score_langs, expected_langs,
        "scores must cover exactly {{overall, java, js_ts}}"
    );
    for lang in EXPECTED_LANGUAGES {
        let lang_obj = scores
            .get(lang)
            .and_then(Value::as_object)
            .with_context(|| format!("scores.{lang} is not an object"))?;
        let lang_keys: BTreeSet<&str> = lang_obj.keys().map(String::as_str).collect();
        let expected_lang_keys: BTreeSet<&str> = EXPECTED_LANGUAGE_KEYS.into_iter().collect();
        assert_eq!(
            lang_keys, expected_lang_keys,
            "scores.{lang}'s key set must be exactly {{erosion, verbosity}}"
        );
        let verbosity = lang_obj
            .get("verbosity")
            .and_then(Value::as_object)
            .with_context(|| format!("scores.{lang}.verbosity is not an object"))?;
        let verbosity_keys: BTreeSet<&str> = verbosity.keys().map(String::as_str).collect();
        assert_eq!(
            verbosity_keys, expected_verbosity_keys,
            "scores.{lang}.verbosity's key set must be exactly {expected_verbosity_keys:?}"
        );
        if verbosity_shape == VerbosityShape::WithCoverage {
            assert!(
                verbosity["complete"].is_boolean(),
                "scores.{lang}.verbosity.complete must be a boolean"
            );
        }
    }

    fn assert_all_leaves_are_numbers(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    assert_all_leaves_are_numbers(nested, &format!("{path}.{key}"));
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    assert_all_leaves_are_numbers(item, &format!("{path}[{index}]"));
                }
            }
            // The one exempt leaf: M1-8's `complete` flag, by exact path
            // shape, so a boolean anywhere else still fails.
            Value::Bool(_)
                if path.starts_with("scores.") && path.ends_with(".verbosity.complete") => {}
            other => assert!(other.is_number(), "{path} is not a number: {other:?}"),
        }
    }

    for field in [
        "scores",
        "clones",
        "findings_by_rule_id",
        "skips_by_reason",
        "top25",
    ] {
        assert_all_leaves_are_numbers(&committed[field], field);
    }

    Ok(())
}

/// Runs unconditionally (no archive needed): the committed M0b digest file
/// itself must carry none of the archived report's private shape.
#[test]
fn test_committed_digest_carries_no_paths_names_or_excerpts() -> Result<()> {
    let committed = read_committed_digest(&committed_digest_path())?;
    assert_digest_carries_no_paths_names_or_excerpts(&committed, VerbosityShape::M0b)
}

/// Runs unconditionally (no archive needed): the committed `nsd-v1` digest
/// file must carry none of the archived report's private shape, exactly
/// like the M0b digest above -- the same forbidden-key, path-shape,
/// top-level-key and numeric-leaf checks, over the new file.
#[test]
fn test_nsd_v1_digest_carries_no_paths_names_or_excerpts() -> Result<()> {
    let committed = read_committed_digest(&nsd_v1_digest_path())?;
    assert_digest_carries_no_paths_names_or_excerpts(&committed, VerbosityShape::WithCoverage)
}

/// A digest still in the three-key M0b verbosity shape is refused under the
/// `nsd-v1` shape, so a recapture that drops M1-8's coverage keys fails.
#[test]
#[should_panic(expected = "verbosity's key set must be exactly")]
fn test_shape_check_rejects_the_m0b_verbosity_under_the_coverage_shape() {
    let m0b = read_committed_digest(&committed_digest_path()).expect("reading the M0b digest");
    let _ = assert_digest_carries_no_paths_names_or_excerpts(&m0b, VerbosityShape::WithCoverage);
}

/// The boolean exemption covers only `scores.*.verbosity.complete`: a
/// boolean anywhere else in the `nsd-v1` digest is still refused.
#[test]
#[should_panic(expected = "clones.group_count is not a number")]
fn test_shape_check_rejects_a_boolean_outside_verbosity_complete() {
    let mut digest =
        read_committed_digest(&nsd_v1_digest_path()).expect("reading the nsd-v1 digest");
    digest["clones"]["group_count"] = Value::Bool(true);
    let _ = assert_digest_carries_no_paths_names_or_excerpts(&digest, VerbosityShape::WithCoverage);
}

/// Unconditional: the M0b and `nsd-v1` committed digests must agree on
/// every field that is not expected to move under the orchard swap -- the
/// fixture's identity and revision, not its measurements.
#[test]
fn test_nsd_v1_digest_shares_the_m0b_revision() -> Result<()> {
    let m0b = read_committed_digest(&committed_digest_path())?;
    let nsd_v1 = read_committed_digest(&nsd_v1_digest_path())?;
    for field in [
        "revision_sha",
        "label",
        "language",
        "authorship",
        "hash_version",
    ] {
        assert_eq!(
            m0b[field], nsd_v1[field],
            "{field} must agree between the M0b and nsd-v1 committed digests"
        );
    }
    Ok(())
}

/// B3 (task.md, ledger row triage-ws4-r1): pins `pending_notice()`'s
/// emission at all four `println!("{}", pending_notice())` call sites
/// in this file (the archived-digest, live-nsd-v1-scan and both bare-pending-gate
/// tests) by observing each one's own stdout from a fresh, re-executed
/// child process, rather than by capturing stdout in-process -- an
/// in-process capture would need a `gag`-style stdout-redirect crate, a new
/// dependency the anti-scope forbids. Each child is `current_exe()` itself,
/// run with `--exact <name> --nocapture` so libtest runs only that one test
/// and does not swallow its stdout, and with `ARCHIVED_REPORT_ENV_VAR` /
/// `REQUIRE_ARCHIVE_VERIFIED_ENV_VAR` removed so it takes the pending
/// branch regardless of what this parent process's own environment
/// happens to carry.
#[test]
fn test_pending_arms_print_the_notice() {
    const PENDING_ARM_TESTS: [&str; 4] = [
        "test_committed_digest_matches_the_archived_report",
        "test_gate_is_reported_pending_when_the_archive_is_absent",
        "test_nsd_v1_live_scan_is_reported_pending_when_the_archive_is_absent",
        "test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01",
    ];
    let binary = std::env::current_exe().expect("the running test binary must have its own path");
    let notice = pending_notice();
    for test_name in PENDING_ARM_TESTS {
        let output = std::process::Command::new(&binary)
            .args(["--exact", test_name, "--nocapture"])
            .env_remove(ARCHIVED_REPORT_ENV_VAR)
            .env_remove(REQUIRE_ARCHIVE_VERIFIED_ENV_VAR)
            .output()
            .unwrap_or_else(|error| panic!("re-executing {test_name} failed: {error}"));
        assert!(
            output.status.success(),
            "{test_name} did not pass in the re-executed child: {output:?}"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&notice),
            "{test_name}'s re-executed child did not print the pending notice: {stdout:?}"
        );
    }
}

/// Pure classification of a possibly-absent `NSD_GOLDEN_CAPTURE` value,
/// split out the same way `classify_archive_gate` and
/// `classify_verification_requirement` are: so `None` and `Some("")`
/// (unset, and set-but-empty) can each be asserted directly as "do not
/// capture", rather than only observed through the process's own
/// environment.
fn classify_golden_capture(raw: Option<OsString>) -> bool {
    matches!(raw, Some(value) if !value.is_empty())
}

fn golden_capture_requested() -> bool {
    classify_golden_capture(std::env::var_os(GOLDEN_CAPTURE_ENV_VAR))
}

#[test]
fn test_classify_golden_capture_needs_a_non_empty_value() {
    assert!(!classify_golden_capture(None));
    assert!(!classify_golden_capture(Some(OsString::new())));
    assert!(classify_golden_capture(Some(OsString::from("1"))));
}

/// What `test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01` needs
/// out of the archived report to reproduce its scan: the same target and
/// scan settings the retired strict leg read
/// (`git show 3dd9ae2:tests/neutrality.rs`,
/// `test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report`),
/// so a private checkout path is read only from the archive at
/// invocation time, never from a repository file.
struct ArchivedScanRecipe {
    target: String,
    include_tests: bool,
    exclude: Vec<String>,
    min_clone_lines: u32,
}

fn read_archived_scan_recipe(archived: &Value) -> Result<ArchivedScanRecipe> {
    let scan = archived
        .get("scan")
        .and_then(Value::as_object)
        .context("archived report is missing a scan object")?;
    let target = scan
        .get("target")
        .and_then(Value::as_str)
        .context("scan.target is missing or not a string")?
        .to_string();
    let include_tests = scan
        .get("include_tests")
        .and_then(Value::as_bool)
        .context("scan.include_tests is missing or not a bool")?;
    let exclude: Vec<String> = scan
        .get("exclude")
        .and_then(Value::as_array)
        .context("scan.exclude is missing or not an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .context("scan.exclude entry is not a string")
        })
        .collect::<Result<_>>()?;
    let min_clone_lines =
        scan.get("min_clone_lines")
            .and_then(Value::as_u64)
            .context("scan.min_clone_lines is missing or not a number")? as u32;
    Ok(ArchivedScanRecipe {
        target,
        include_tests,
        exclude,
        min_clone_lines,
    })
}

/// Proves the pending path is reachable and prints its notice rather than
/// silently substituting a pass, independent of whether this particular
/// invocation happens to carry the archive-backed leg too -- mirroring
/// `test_gate_is_reported_pending_when_the_archive_is_absent` for the
/// live-scan leg below.
#[test]
fn test_nsd_v1_live_scan_is_reported_pending_when_the_archive_is_absent() {
    match archive_gate() {
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
        }
        ArchiveGate::Resolved(_) => {
            // The archive-backed leg is running in this invocation; the
            // pending branch above is exercised by this same test in the
            // ordinary (archive-absent) developer/CI run instead.
        }
    }
}

/// Whether the git checkout holding `target` has no modified or untracked
/// entries. Panics (details withheld) when no repository can be opened.
fn archive_checkout_is_clean(target: &Path) -> bool {
    let checkout = git2::Repository::discover(target).unwrap_or_else(|_| {
        panic!(
            "opening the private java-fixture-01 checkout failed; details withheld \
             (AGENTS.md, Fixture privacy)"
        )
    });
    let mut status_options = git2::StatusOptions::new();
    status_options.include_untracked(true);
    let statuses = checkout
        .statuses(Some(&mut status_options))
        .unwrap_or_else(|_| {
            panic!(
                "reading the private java-fixture-01 checkout status failed; details \
                 withheld (AGENTS.md, Fixture privacy)"
            )
        });
    statuses.is_empty()
}

/// A recipe target that is a subdirectory of a clean checkout passes the
/// cleanliness check, a dirty checkout still fails it, and a target outside
/// any repository is refused loudly (ledger row 124).
#[test]
fn test_archive_cleanliness_accepts_a_subdirectory_target() -> Result<()> {
    let repo_dir = tempfile::tempdir().context("creating the repository tempdir")?;
    let repo = git2::Repository::init(repo_dir.path()).context("initialising the repository")?;
    let target = repo_dir.path().join("module");
    fs::create_dir(&target).context("creating the target subdirectory")?;
    fs::write(target.join("A.java"), "class A {}\n").context("writing a tracked file")?;
    let mut index = repo.index().context("opening the index")?;
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .context("staging the file")?;
    index.write().context("writing the index")?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let signature = git2::Signature::now("nsd test", "nsd@example.invalid")?;
    repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])?;

    assert!(
        archive_checkout_is_clean(&target),
        "a subdirectory of a clean checkout must pass the cleanliness check"
    );

    fs::write(target.join("B.java"), "class B {}\n").context("writing an untracked file")?;
    assert!(
        !archive_checkout_is_clean(&target),
        "an untracked file must make the checkout dirty"
    );

    let outside = tempfile::tempdir().context("creating the non-repository tempdir")?;
    let refused = std::panic::catch_unwind(|| archive_checkout_is_clean(outside.path()));
    assert!(
        refused.is_err(),
        "a target outside any repository must be refused"
    );
    Ok(())
}

/// D15: re-scans the archive's own recorded target with its own recorded
/// settings -- exactly as the retired strict leg did -- and asserts the
/// live `nsd-v1` digest equals the committed `tests/golden/java-fixture-01.
/// nsd-v1.digest.json`. The archive is resolved only from
/// `NSD_ARCHIVED_REPORT` at invocation time and never committed
/// (`AGENTS.md`, *Fixture privacy*); the freshly rendered `report.json`
/// (which does carry real source excerpts, for a real local checkout
/// target) is written only to a tempdir outside this repository and never
/// read past `golden::build_digest`'s own excerpt strip.
///
/// A missing archive never invents a pass here, same as the M0b check
/// above: the pending arm asserts nothing. `NSD_GOLDEN_CAPTURE` (checked
/// only once the archive is resolved) turns this from a comparison into
/// an implementer-only capture that writes the live digest to the
/// committed file instead of asserting equality.
#[test]
fn test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01() -> Result<()> {
    let path = match archive_gate() {
        ArchiveGate::Resolved(path) => path,
        ArchiveGate::Pending => {
            println!("{}", pending_notice());
            if verification_required() {
                panic!("{}", required_but_pending_message());
            }
            return Ok(());
        }
    };

    let archived_raw = fs::read_to_string(&path).context("reading the archived report")?;
    let archived = golden::parse_report(&archived_raw).context("parsing the archived report")?;
    let recipe = read_archived_scan_recipe(&archived)?;

    let output_dir = tempfile::tempdir().context("creating the live-scan output tempdir")?;
    let settings = ScanSettings {
        output: output_dir.path().to_path_buf(),
        include_tests: recipe.include_tests,
        exclude: recipe.exclude,
        min_clone_lines: recipe.min_clone_lines,
    };
    pipeline::run(&recipe.target, settings).unwrap_or_else(|_| {
        panic!(
            "running the live scan of java-fixture-01 failed; details withheld \
             (AGENTS.md, Fixture privacy)"
        )
    });
    let live_raw = fs::read_to_string(output_dir.path().join("report.json"))
        .context("reading the freshly rendered live report.json")?;
    let mut live = golden::parse_report(&live_raw).context("parsing the live report")?;

    let committed_m0b = read_committed_digest(&committed_digest_path())?;
    let expected_revision_sha = committed_m0b["revision_sha"]
        .as_str()
        .context("committed M0b digest is missing revision_sha")?;
    let live_sha = live["scan"]["revision"]["sha"]
        .as_str()
        .context("live report is missing scan.revision.sha")?;
    assert_eq!(
        live_sha, expected_revision_sha,
        "the private java-fixture-01 checkout is not at the recorded revision; \
         details withheld (AGENTS.md, Fixture privacy)"
    );
    assert!(
        live["scan"]["revision"]["dirty"].is_null(),
        "a local scan must report scan.revision.dirty as null"
    );
    assert!(
        archive_checkout_is_clean(Path::new(&recipe.target)),
        "the private java-fixture-01 checkout is dirty; details withheld \
         (AGENTS.md, Fixture privacy)"
    );
    let live_incomplete = live["incomplete"]
        .as_bool()
        .context("live report is missing incomplete")?;
    assert!(
        !live_incomplete,
        "the live scan of java-fixture-01 is incomplete"
    );

    let (live_digest, _removed) =
        golden::build_digest(&mut live).context("building the live nsd-v1 digest")?;

    let nsd_v1_path = nsd_v1_digest_path();
    if golden_capture_requested() {
        let text = serde_json::to_string_pretty(&live_digest)
            .context("serializing the live nsd-v1 digest")?
            + "\n";
        fs::write(&nsd_v1_path, text).context("writing the nsd-v1 golden digest")?;
        return Ok(());
    }

    let committed_nsd_v1 = read_committed_digest(&nsd_v1_path)?;
    assert_eq!(
        live_digest, committed_nsd_v1,
        "the live nsd-v1 scan of java-fixture-01 no longer matches the committed digest"
    );
    Ok(())
}

//! M0c-12: freeze the `nsd-v1` measurement fingerprint (`nsd-plan-final.md`
//! M0c step 12, *Settled decisions* → *Measurement profile*, *Corrections
//! applied* #18). A fingerprint over exactly the inputs the plan names —
//! the IR version, both per-language lowering versions, the three grammar
//! versions, the `tree-sitter` runtime version, the rule catalog, and the
//! effective clone configuration's `min_clone_lines` — so a future change
//! to any one of them is forced to move the fingerprint rather than
//! silently reuse a cache key or golden digest computed under a different
//! profile. No `report.json` field and no cache read or write here: M5's
//! cache key and M6-2's canonical-JSON fingerprint field are later
//! consumers of `fingerprint`, not built by this module.

use crate::hashing::Digest;
use crate::ir;
use crate::lower;
use crate::model::{RuleId, DEFAULT_MIN_CLONE_LINES};
use crate::rules;

/// The name of this frozen profile, distinct from the version constants
/// below: it identifies *which* profile a fingerprint was computed under,
/// the way a schema name accompanies a schema hash.
pub const PROFILE_NAME: &str = "nsd-v1";

/// Pinned against `Cargo.lock`'s `tree-sitter` package by
/// `tests/profile.rs::test_version_constants_match_cargo_lock` — this is
/// the runtime crate version, not either grammar's own version and not
/// `tree_sitter::LANGUAGE_VERSION`/`MIN_COMPATIBLE_LANGUAGE_VERSION` (those
/// are ABI numbers, a different thing from the crate version the plan
/// means by "the `tree-sitter` runtime version").
pub const TREE_SITTER_RUNTIME_VERSION: &str = "0.27.0";

/// Pinned against `Cargo.lock`'s `tree-sitter-java-orchard` package.
pub const JAVA_GRAMMAR_VERSION: &str = "0.5.18";

/// Pinned against `Cargo.lock`'s `tree-sitter-javascript` package.
pub const JAVASCRIPT_GRAMMAR_VERSION: &str = "0.25.0";

/// Pinned against `Cargo.lock`'s `tree-sitter-typescript` package.
pub const TYPESCRIPT_GRAMMAR_VERSION: &str = "0.23.2";

/// The BLAKE3 family-prefix domain `fingerprint` hashes under, distinct
/// from `src/golden.rs`'s `"golden-digest-body"` prefix and from
/// `src/clones/mod.rs`'s per-language prefixes, so a measurement
/// fingerprint can never collide with either even on identical input
/// bytes.
const FINGERPRINT_FAMILY_PREFIX: &str = "measurement-profile";

/// Exactly the inputs `nsd-plan-final.md` M0c step 12 names for the
/// measurement fingerprint — nothing else. `current()` is the only
/// constructor v1 callers need; the fields stay public so
/// `tests/profile.rs` can build deliberately altered copies to prove each
/// one actually moves `fingerprint`'s output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurementProfileInputs {
    pub ir_version: u32,
    pub java_lowering_version: u32,
    pub jsts_lowering_version: u32,
    pub tree_sitter_runtime_version: &'static str,
    pub java_grammar_version: &'static str,
    pub javascript_grammar_version: &'static str,
    pub typescript_grammar_version: &'static str,
    pub rule_catalog: &'static [RuleId],
    pub min_clone_lines: u32,
}

impl MeasurementProfileInputs {
    /// The live inputs for a default-configuration scan: every version
    /// constant read off its own source of truth (`ir::IR_VERSION`,
    /// `lower::JAVA_LOWERING_VERSION`, `lower::JSTS_LOWERING_VERSION`, the
    /// four constants above) plus `rules::ALL_RULE_IDS` and
    /// `model::DEFAULT_MIN_CLONE_LINES`, so this struct can never drift
    /// from the constants those modules already bump on their own change.
    pub fn current() -> Self {
        Self {
            ir_version: ir::IR_VERSION,
            java_lowering_version: lower::JAVA_LOWERING_VERSION,
            jsts_lowering_version: lower::JSTS_LOWERING_VERSION,
            tree_sitter_runtime_version: TREE_SITTER_RUNTIME_VERSION,
            java_grammar_version: JAVA_GRAMMAR_VERSION,
            javascript_grammar_version: JAVASCRIPT_GRAMMAR_VERSION,
            typescript_grammar_version: TYPESCRIPT_GRAMMAR_VERSION,
            rule_catalog: &rules::ALL_RULE_IDS,
            min_clone_lines: DEFAULT_MIN_CLONE_LINES,
        }
    }
}

/// The versioned-BLAKE3 measurement fingerprint over `inputs`, algorithm-
/// prefixed lowercase hex (the same `"blake3:{:032x}"` shape
/// `src/golden.rs::body_blake3` commits). Identical inputs always yield
/// identical output; every field in `MeasurementProfileInputs` is pushed
/// into the digest, so changing any one of them changes this value
/// (`tests/profile.rs::test_each_input_changes_the_fingerprint`).
pub fn fingerprint(inputs: &MeasurementProfileInputs) -> String {
    let mut digest = Digest::new(FINGERPRINT_FAMILY_PREFIX);
    digest.push(&inputs.ir_version.to_le_bytes());
    digest.push(&inputs.java_lowering_version.to_le_bytes());
    digest.push(&inputs.jsts_lowering_version.to_le_bytes());
    digest.push(inputs.tree_sitter_runtime_version.as_bytes());
    digest.push(inputs.java_grammar_version.as_bytes());
    digest.push(inputs.javascript_grammar_version.as_bytes());
    digest.push(inputs.typescript_grammar_version.as_bytes());
    for rule_id in inputs.rule_catalog {
        digest.push(rule_id.as_bytes());
    }
    digest.push(&inputs.min_clone_lines.to_le_bytes());
    format!("blake3:{:032x}", digest.finish())
}

/// The fingerprint of a run measuring with `min_clone_lines` (a check's
/// trusted `measurement.min_clone_lines`, a scan's `--min-clone-lines`): the
/// live profile with only that threshold replaced. Never the bare
/// `MeasurementProfileInputs::current()`, whose threshold is the default.
pub fn measurement_fingerprint(min_clone_lines: u32) -> String {
    fingerprint(&MeasurementProfileInputs {
        min_clone_lines,
        ..MeasurementProfileInputs::current()
    })
}

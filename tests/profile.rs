//! M0c-12: the frozen `nsd-v1` measurement fingerprint (`src/profile.rs`).
//! `nsd-plan-final.md` M0c step 12: the fingerprint covers IR version, both
//! lowering versions, the three grammar versions, the `tree-sitter` runtime
//! version, the rule catalog, and the effective clone configuration
//! (`min_clone_lines`) — nothing else, and every one of those inputs must
//! actually move the fingerprint.

use nsd::profile::{self, MeasurementProfileInputs};

/// Reads the `version = "..."` pinned for `crate_name` in this repo's own
/// `Cargo.lock`, the same way `src/profile.rs`'s constants must be kept in
/// sync (`test_version_constants_match_cargo_lock` below): a plain string
/// search, not a TOML parser, since M0c-12 adds no new dependency.
fn cargo_lock_version(crate_name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let lock_path = std::path::Path::new(manifest_dir).join("Cargo.lock");
    let contents = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", lock_path.display()));
    let name_needle = format!("name = \"{crate_name}\"\n");
    let name_pos = contents
        .find(&name_needle)
        .unwrap_or_else(|| panic!("Cargo.lock has no `{crate_name}` package"));
    let after_name = &contents[name_pos + name_needle.len()..];
    let version_needle = "version = \"";
    let version_pos = after_name
        .find(version_needle)
        .unwrap_or_else(|| panic!("no version line after `{crate_name}`'s name line"));
    let after_version = &after_name[version_pos + version_needle.len()..];
    let end = after_version
        .find('"')
        .unwrap_or_else(|| panic!("unterminated version string for `{crate_name}`"));
    after_version[..end].to_string()
}

#[test]
fn test_version_constants_match_cargo_lock() {
    assert_eq!(
        profile::TREE_SITTER_RUNTIME_VERSION,
        cargo_lock_version("tree-sitter"),
        "profile::TREE_SITTER_RUNTIME_VERSION is stale against Cargo.lock"
    );
    assert_eq!(
        profile::JAVA_GRAMMAR_VERSION,
        cargo_lock_version("tree-sitter-java-orchard"),
        "profile::JAVA_GRAMMAR_VERSION is stale against Cargo.lock"
    );
    assert_eq!(
        profile::JAVASCRIPT_GRAMMAR_VERSION,
        cargo_lock_version("tree-sitter-javascript"),
        "profile::JAVASCRIPT_GRAMMAR_VERSION is stale against Cargo.lock"
    );
    assert_eq!(
        profile::TYPESCRIPT_GRAMMAR_VERSION,
        cargo_lock_version("tree-sitter-typescript"),
        "profile::TYPESCRIPT_GRAMMAR_VERSION is stale against Cargo.lock"
    );
}

#[test]
fn test_current_inputs_read_the_live_constants() {
    let current = MeasurementProfileInputs::current();
    assert_eq!(current.ir_version, nsd::ir::IR_VERSION);
    assert_eq!(
        current.java_lowering_version,
        nsd::lower::JAVA_LOWERING_VERSION
    );
    assert_eq!(
        current.jsts_lowering_version,
        nsd::lower::JSTS_LOWERING_VERSION
    );
    assert_eq!(current.rule_catalog, &nsd::rules::ALL_RULE_IDS);
    assert_eq!(current.min_clone_lines, nsd::model::DEFAULT_MIN_CLONE_LINES);
    assert_eq!(
        current.tree_sitter_runtime_version,
        profile::TREE_SITTER_RUNTIME_VERSION
    );
    assert_eq!(current.java_grammar_version, profile::JAVA_GRAMMAR_VERSION);
    assert_eq!(
        current.javascript_grammar_version,
        profile::JAVASCRIPT_GRAMMAR_VERSION
    );
    assert_eq!(
        current.typescript_grammar_version,
        profile::TYPESCRIPT_GRAMMAR_VERSION
    );
}

#[test]
fn test_identical_inputs_give_identical_fingerprints() {
    let a = MeasurementProfileInputs::current();
    let b = MeasurementProfileInputs::current();
    assert_eq!(profile::fingerprint(&a), profile::fingerprint(&b));
}

/// "changing the IR version, either lowering version, or any grammar
/// version changes the measurement fingerprint" (`nsd-plan-final.md`, *Test
/// and acceptance plan*), extended here to the three inputs the *Settled
/// decisions* / *Corrections applied* text adds on top of that list: the
/// `tree-sitter` runtime version, the rule catalog, and
/// `measurement.min_clone_lines` (`nsd-plan-implementation.md`).
#[test]
fn test_each_input_changes_the_fingerprint() {
    let baseline = MeasurementProfileInputs::current();
    let baseline_fingerprint = profile::fingerprint(&baseline);

    let mut changed = baseline;
    changed.ir_version += 1;
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "ir_version"
    );

    let mut changed = baseline;
    changed.java_lowering_version += 1;
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "java_lowering_version"
    );

    let mut changed = baseline;
    changed.jsts_lowering_version += 1;
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "jsts_lowering_version"
    );

    let mut changed = baseline;
    changed.tree_sitter_runtime_version = "0.27.1";
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "tree_sitter_runtime_version"
    );

    let mut changed = baseline;
    changed.java_grammar_version = "0.5.19";
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "java_grammar_version"
    );

    let mut changed = baseline;
    changed.javascript_grammar_version = "0.25.1";
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "javascript_grammar_version"
    );

    let mut changed = baseline;
    changed.typescript_grammar_version = "0.23.3";
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "typescript_grammar_version"
    );

    let mut changed = baseline;
    changed.rule_catalog = &nsd::rules::ALL_RULE_IDS[..nsd::rules::ALL_RULE_IDS.len() - 1];
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "rule_catalog"
    );

    let mut changed = baseline;
    changed.min_clone_lines += 1;
    assert_ne!(
        profile::fingerprint(&changed),
        baseline_fingerprint,
        "min_clone_lines"
    );
}

/// The freeze itself: the default-configuration fingerprint is pinned
/// against a committed hex literal. A future change to any input above
/// must fail this assertion first, and update it deliberately (with a
/// recorded reason), rather than silently drift.
#[test]
fn test_nsd_v1_fingerprint_is_frozen() {
    // Re-pinned for M1-7: `ir::IR_VERSION` moved 3 -> 4 (`IrCallable` gains
    // `kind`/`is_anonymous`/`signature`/`owner_chain`), which is one of the
    // nine hashed inputs (`MeasurementProfileInputs::ir_version`). The name
    // stays `nsd-v1` (`docs/measurement-profile.md` "The bump rule" and
    // "What `PROFILE_NAME` names" -- the name identifies the profile
    // generation, not any one literal); only the hash moves.
    let fingerprint = profile::fingerprint(&MeasurementProfileInputs::current());
    assert_eq!(
        fingerprint, "blake3:539bfba3a29d159bb0f3f7239931bdc8",
        "nsd-v1's frozen fingerprint moved — bump this literal deliberately, with a reason, \
         if the input that moved it is an intentional profile change"
    );
}

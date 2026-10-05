//! Integration tests for M4-1's A102 required analysis coverage
//! (`nsd-plan-final.md` *Required clone coverage*). Every scenario commits
//! real sources to a scratch repository and goes through real discovery, diff
//! and analysis.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use git2::{Oid, Repository};
use nsd::analysis::{analyze_file, UnanalyzableReason};
use nsd::config::{Config, PolicyConfig};
use nsd::git::diff::{diff_commit_to_commit, Change};
use nsd::git::discovery::discover;
use nsd::git::path::RepoPath;
use nsd::git::snapshot::{CommitSnapshot, SOURCE_CEILING_BYTES};
use nsd::policy::coverage::{evaluate_coverage, CoverageInput};
use nsd::policy::diagnostics::{CoverageDiagnostic, CODE_ANALYSIS_UNAVAILABLE};

const MODE_REGULAR: i32 = 0o100644;
const SMALL: &str = "class S { void m() { return; } }\n";
const SMALL_EDITED: &str = "class S { void m() { return; } }\n// edited\n";
const LEGACY: &str = "class W {\n    void ok1() {\n        a();\n    }\n    void broken(int a {\n        return;\n    }\n    void ok2() {\n        b();\n    }\n}\n";

fn config(yaml: &str) -> Config {
    Config::parse(yaml.as_bytes()).expect("a valid nsd.yml")
}

fn policy_with(v102: &str) -> PolicyConfig {
    config(&format!("version: 1\npolicy:\n  NSD-V102: {v102}\n")).policy
}

fn padded_java(total_bytes: u64) -> Vec<u8> {
    let mut source = b"class Big { void m() { return; } }".to_vec();
    source.resize(total_bytes as usize, b' ');
    source
}

fn commit(repo: &Repository, files: &[(&[u8], Vec<u8>)]) -> Oid {
    let entries: Vec<_> = files
        .iter()
        .map(|(path, bytes)| (path.to_vec(), MODE_REGULAR, bytes.clone()))
        .collect();
    common::commit_entries(repo, &entries)
}

fn candidate_side_paths(changes: &[Change]) -> BTreeSet<RepoPath> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Added { path, .. }
            | Change::Modified { path, .. }
            | Change::Typechange { path, .. } => Some(path.clone()),
            Change::Renamed { to, .. } => Some(to.clone()),
            Change::Deleted { .. } => None,
        })
        .collect()
}

/// Commits `base_files`, then `candidate_files`, and returns the coverage
/// inputs a caller would build: discovery under `config`, `changed` from the
/// diff, `failure` from `analyze_file` when the bytes were read.
fn inputs_for(
    base_files: &[(&[u8], Vec<u8>)],
    candidate_files: &[(&[u8], Vec<u8>)],
    config: &Config,
) -> Vec<CoverageInput> {
    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, base_files);
    let candidate = commit(&repo, candidate_files);
    let changed = candidate_side_paths(
        &diff_commit_to_commit(&repo, Some(base), candidate).expect("diff the commits"),
    );
    let snapshot = CommitSnapshot::head_or_empty(&repo).expect("snapshot HEAD");
    let scope = config.compiled_scope().expect("the config compiles");
    let result = discover(&snapshot.entries, &scope);
    result
        .included
        .into_iter()
        .map(|entry| {
            let source = snapshot
                .entries
                .iter()
                .find(|candidate_entry| candidate_entry.path == entry.path)
                .and_then(|found| snapshot.read(&repo, found).expect("read blob"));
            let failure = source
                .and_then(|bytes| analyze_file(Path::new(&entry.path.render()), &bytes).err());
            CoverageInput {
                changed: changed.contains(&entry.path),
                entry,
                failure,
            }
        })
        .collect()
}

fn named(diagnostics: &[CoverageDiagnostic]) -> Vec<(String, UnanalyzableReason)> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            assert_eq!(diagnostic.code, CODE_ANALYSIS_UNAVAILABLE);
            (diagnostic.path.render(), diagnostic.reason)
        })
        .collect()
}

fn large_unchanged_scenario(config: &Config) -> Vec<CoverageInput> {
    let big = padded_java(SOURCE_CEILING_BYTES + 1);
    inputs_for(
        &[
            (b"Big.java", big.clone()),
            (b"Small.java", SMALL.as_bytes().to_vec()),
        ],
        &[
            (b"Big.java", big),
            (b"Small.java", SMALL_EDITED.as_bytes().to_vec()),
        ],
        config,
    )
}

#[test]
fn test_unchanged_oversized_clone_input_raises_a102_unless_v102_is_off() {
    let inputs = large_unchanged_scenario(&Config::default());
    for severity in ["deny", "warn"] {
        let diagnostics = evaluate_coverage(&inputs, &policy_with(severity));
        assert_eq!(
            named(&diagnostics),
            vec![("Big.java".to_string(), UnanalyzableReason::TooLarge)],
            "{severity}"
        );
    }
    assert!(evaluate_coverage(&inputs, &policy_with("off")).is_empty());
}

#[test]
fn test_changed_oversized_file_raises_a102_even_with_v102_off() {
    let big = padded_java(SOURCE_CEILING_BYTES + 1);
    let mut inputs = inputs_for(
        &[(b"Small.java", SMALL.as_bytes().to_vec())],
        &[(b"Big.java", big)],
        &Config::default(),
    );
    assert!(inputs[0].changed);
    let diagnostics = evaluate_coverage(&inputs, &policy_with("off"));
    assert_eq!(
        named(&diagnostics),
        vec![("Big.java".to_string(), UnanalyzableReason::TooLarge)]
    );

    inputs[0].changed = false;
    assert!(evaluate_coverage(&inputs, &policy_with("off")).is_empty());
    assert_eq!(evaluate_coverage(&inputs, &policy_with("deny")).len(), 1);
}

#[test]
fn test_exactly_one_mebibyte_raises_nothing() {
    let exact = padded_java(SOURCE_CEILING_BYTES);
    let inputs = inputs_for(
        &[(b"Exact.java", exact.clone())],
        &[
            (b"Exact.java", exact),
            (b"Small.java", SMALL.as_bytes().to_vec()),
        ],
        &Config::default(),
    );
    assert_eq!(inputs.len(), 2);
    assert!(evaluate_coverage(&inputs, &policy_with("deny")).is_empty());
}

#[test]
fn test_invalid_encoding_raises_a102_for_changed_and_required_unchanged_files() {
    let invalid = b"export const s = \"\xff\xfe\";\n".to_vec();
    let inputs = inputs_for(
        &[(b"stale.ts", invalid.clone())],
        &[(b"stale.ts", invalid.clone()), (b"fresh.ts", invalid)],
        &Config::default(),
    );
    let expected = vec![
        ("fresh.ts".to_string(), UnanalyzableReason::InvalidEncoding),
        ("stale.ts".to_string(), UnanalyzableReason::InvalidEncoding),
    ];
    for severity in ["deny", "warn"] {
        assert_eq!(
            named(&evaluate_coverage(&inputs, &policy_with(severity))),
            expected,
            "{severity}"
        );
    }
    assert_eq!(
        named(&evaluate_coverage(&inputs, &policy_with("off"))),
        vec![("fresh.ts".to_string(), UnanalyzableReason::InvalidEncoding)]
    );
}

#[cfg(unix)]
#[test]
fn test_non_utf8_path_raises_a102() {
    let inputs = inputs_for(
        &[(b"src/bad\xff.java", SMALL.as_bytes().to_vec())],
        &[(b"src/bad\xff.java", SMALL.as_bytes().to_vec())],
        &Config::default(),
    );
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].entry.non_utf8_path);
    assert_eq!(
        named(&evaluate_coverage(&inputs, &policy_with("deny"))),
        vec![(
            "src/bad%FF.java".to_string(),
            UnanalyzableReason::NonUtf8Path
        )]
    );
    assert!(evaluate_coverage(&inputs, &policy_with("off")).is_empty());
}

#[test]
fn test_capability_failure_raises_a102() {
    let mut inputs = inputs_for(
        &[(b"Small.java", SMALL.as_bytes().to_vec())],
        &[(b"Small.java", SMALL.as_bytes().to_vec())],
        &Config::default(),
    );
    inputs[0].failure = Some(UnanalyzableReason::ParserUnavailable);
    for severity in ["deny", "warn"] {
        assert_eq!(
            named(&evaluate_coverage(&inputs, &policy_with(severity))),
            vec![(
                "Small.java".to_string(),
                UnanalyzableReason::ParserUnavailable
            )],
            "{severity}"
        );
    }
    inputs[0].changed = true;
    assert_eq!(evaluate_coverage(&inputs, &policy_with("off")).len(), 1);
}

#[test]
fn test_trusted_exclusion_removes_the_obligation() {
    let big = padded_java(SOURCE_CEILING_BYTES + 1);
    let files: &[(&[u8], Vec<u8>)] = &[
        (b"legacy/Big.java", big),
        (b"Small.java", SMALL.as_bytes().to_vec()),
    ];
    let excluded = config("version: 1\nexclude: [\"legacy/**\"]\n");
    let inputs = inputs_for(files, files, &excluded);
    assert_eq!(inputs.len(), 1);
    assert!(evaluate_coverage(&inputs, &policy_with("deny")).is_empty());

    let kept = inputs_for(files, files, &Config::default());
    assert_eq!(
        named(&evaluate_coverage(&kept, &policy_with("deny"))),
        vec![("legacy/Big.java".to_string(), UnanalyzableReason::TooLarge)]
    );
}

#[test]
fn test_legacy_parse_damage_is_not_a102() {
    let inputs = inputs_for(
        &[(b"Legacy.java", LEGACY.as_bytes().to_vec())],
        &[
            (b"Legacy.java", LEGACY.as_bytes().to_vec()),
            (b"Small.java", SMALL.as_bytes().to_vec()),
        ],
        &Config::default(),
    );
    assert!(inputs.iter().all(|input| input.failure.is_none()));
    assert!(evaluate_coverage(&inputs, &policy_with("deny")).is_empty());
}

fn unsupported_extension_input(changed: bool) -> Vec<CoverageInput> {
    let mut inputs = inputs_for(
        &[(b"Small.java", SMALL.as_bytes().to_vec())],
        &[(b"Small.java", SMALL.as_bytes().to_vec())],
        &Config::default(),
    );
    inputs[0].failure = Some(UnanalyzableReason::UnsupportedExtension);
    inputs[0].changed = changed;
    inputs
}

/// Ledger row 108 (user decision): a changed included input analysis
/// reports as `UnsupportedExtension` fails closed, whatever V102 says.
#[test]
fn test_changed_unsupported_extension_raises_a102() {
    let inputs = unsupported_extension_input(true);
    for v102 in ["deny", "off"] {
        assert_eq!(
            named(&evaluate_coverage(&inputs, &policy_with(v102))),
            vec![(
                "Small.java".to_string(),
                UnanalyzableReason::UnsupportedExtension
            )],
            "V102 {v102}"
        );
    }
}

#[test]
fn test_unchanged_unsupported_extension_raises_no_a102() {
    let inputs = unsupported_extension_input(false);
    assert!(evaluate_coverage(&inputs, &policy_with("deny")).is_empty());
}

#[test]
fn test_output_is_sorted_by_path() {
    let big = padded_java(SOURCE_CEILING_BYTES + 1);
    let files: &[(&[u8], Vec<u8>)] = &[
        (b"b/Big.java", big.clone()),
        (b"a/Big.java", big.clone()),
        (b"c/Big.java", big),
    ];
    let mut inputs = inputs_for(files, files, &Config::default());
    inputs.reverse();
    let paths: Vec<String> = evaluate_coverage(&inputs, &policy_with("deny"))
        .iter()
        .map(|diagnostic| diagnostic.path.render())
        .collect();
    assert_eq!(paths, vec!["a/Big.java", "b/Big.java", "c/Big.java"]);
}

#[test]
fn test_config_exclude_removes_an_invalid_utf8_ts_file_from_a102() {
    let invalid = b"export const s = \"\xff\xfe\";\n".to_vec();
    let files: &[(&[u8], Vec<u8>)] = &[
        (b"legacy/bad.ts", invalid),
        (b"Small.java", SMALL.as_bytes().to_vec()),
    ];
    let excluded = config("version: 1\nexclude: [\"legacy/**\"]\n");
    let inputs = inputs_for(files, files, &excluded);
    assert_eq!(inputs.len(), 1);
    assert!(evaluate_coverage(&inputs, &policy_with("deny")).is_empty());

    let kept = inputs_for(files, files, &Config::default());
    assert_eq!(
        named(&evaluate_coverage(&kept, &policy_with("deny"))),
        vec![(
            "legacy/bad.ts".to_string(),
            UnanalyzableReason::InvalidEncoding
        )]
    );
}

#[cfg(unix)]
#[test]
fn test_too_large_takes_precedence_over_non_utf8_path() {
    let big = padded_java(SOURCE_CEILING_BYTES + 1);
    let files: &[(&[u8], Vec<u8>)] = &[(b"src/bad\xff.java", big)];
    let inputs = inputs_for(files, files, &Config::default());
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].entry.too_large && inputs[0].entry.non_utf8_path);
    assert_eq!(
        named(&evaluate_coverage(&inputs, &policy_with("deny"))),
        vec![("src/bad%FF.java".to_string(), UnanalyzableReason::TooLarge)]
    );
}

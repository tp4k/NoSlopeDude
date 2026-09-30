//! Integration tests for M3-1's E101/E102 classification and G102
//! (`nsd-plan-final.md` *Diagnostics* and *Stable data model*). Boundary and
//! ambiguity scenarios use synthetic metrics fed through the real matcher;
//! the commit-to-commit scenarios go through real parsing and `git2`.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use git2::{Oid, Repository};
use nsd::analysis::analyze_file;
use nsd::config::{Config, PolicyConfig, Severity};
use nsd::git::diff::{diff_commit_to_commit, Change};
use nsd::git::path::RepoPath;
use nsd::git::snapshot::CommitSnapshot;
use nsd::identity::matching::{match_callables, FileCallables};
use nsd::identity::{CallableIdentity, OwnerDigest};
use nsd::ir::CallableKind;
use nsd::model::{Callable, LanguageFamily};
use nsd::policy::complexity::{classify, FileMetrics};
use nsd::policy::diagnostics::{
    PolicyDiagnostic, CODE_COMPLEXITY_ABOVE_THRESHOLD, CODE_COMPLEXITY_INCREASED,
    CODE_MATCH_AMBIGUITY,
};

const MODE_REGULAR: i32 = 0o100644;
const BOUND: Duration = Duration::from_secs(30);
const LARGE_GROUP: usize = 100_000;

// ---------------------------------------------------------------------
// Synthetic scenarios.
// ---------------------------------------------------------------------

struct Spec {
    name: String,
    fingerprint: String,
    cc: u32,
    sloc: usize,
}

fn spec(name: &str, fingerprint: &str, cc: u32, sloc: usize) -> Spec {
    Spec {
        name: name.to_string(),
        fingerprint: fingerprint.to_string(),
        cc,
        sloc,
    }
}

fn identity(name: &str) -> CallableIdentity {
    CallableIdentity {
        owner_digest: OwnerDigest::default(),
        kind: CallableKind::JavaMethod,
        name: name.to_string(),
        signature: Vec::new(),
    }
}

fn path(text: &str) -> RepoPath {
    RepoPath::from_bytes(text.as_bytes().to_vec())
}

fn side(file: &str, specs: &[Spec]) -> (FileCallables, FileMetrics) {
    let callables = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| Callable {
            relative_path: PathBuf::from(file),
            language: LanguageFamily::Java,
            name: spec.name.clone(),
            start_line: index + 1,
            end_line: index + 1,
            cc: spec.cc,
            sloc: spec.sloc,
            mass: 0.0,
        })
        .collect();
    (
        FileCallables {
            path: path(file),
            callables: specs
                .iter()
                .map(|spec| (identity(&spec.name), spec.fingerprint.clone()))
                .collect(),
        },
        FileMetrics {
            path: path(file),
            callables,
        },
    )
}

fn sides(files: &[(&str, Vec<Spec>)]) -> (Vec<FileCallables>, Vec<FileMetrics>) {
    files.iter().map(|(file, specs)| side(file, specs)).unzip()
}

fn default_policy() -> PolicyConfig {
    Config::default().policy
}

fn run_with(
    base: &[(&str, Vec<Spec>)],
    candidate: &[(&str, Vec<Spec>)],
    policy: &PolicyConfig,
) -> Vec<PolicyDiagnostic> {
    let (base_callables, base_metrics) = sides(base);
    let (candidate_callables, candidate_metrics) = sides(candidate);
    let matched = match_callables(&base_callables, &candidate_callables, &[]);
    classify(&base_metrics, &candidate_metrics, &matched, policy)
}

fn run(base: &[(&str, Vec<Spec>)], candidate: &[(&str, Vec<Spec>)]) -> Vec<PolicyDiagnostic> {
    run_with(base, candidate, &default_policy())
}

fn codes(diagnostics: &[PolicyDiagnostic]) -> Vec<(&'static str, &str)> {
    diagnostics
        .iter()
        .map(|d| (d.code, d.candidate_name.as_str()))
        .collect()
}

/// One same-key method `m` edited in place (fingerprint changes) in `A.java`.
fn edited(base_cc: u32, base_sloc: usize, cc: u32, sloc: usize) -> Vec<PolicyDiagnostic> {
    run(
        &[("A.java", vec![spec("m", "before", base_cc, base_sloc)])],
        &[("A.java", vec![spec("m", "after", cc, sloc)])],
    )
}

// ---------------------------------------------------------------------
// E101 / E102.
// ---------------------------------------------------------------------

#[test]
fn test_added_callable_above_threshold_raises_e101() {
    let diagnostics = run(&[], &[("A.java", vec![spec("m", "f", 11, 5)])]);

    assert_eq!(
        codes(&diagnostics),
        vec![(CODE_COMPLEXITY_ABOVE_THRESHOLD, "m")]
    );
    assert_eq!(diagnostics[0].candidate_path, path("A.java"));
    assert_eq!(diagnostics[0].base, None);
}

#[test]
fn test_cc_10_is_not_above_threshold() {
    assert!(run(&[], &[("A.java", vec![spec("m", "f", 10, 5)])]).is_empty());
    assert!(edited(10, 5, 10, 5).is_empty());
}

#[test]
fn test_crossing_the_threshold_raises_e101() {
    let diagnostics = edited(10, 5, 11, 5);

    assert_eq!(
        codes(&diagnostics),
        vec![(CODE_COMPLEXITY_ABOVE_THRESHOLD, "m")]
    );
    let base = diagnostics[0].base.as_ref().expect("a paired E101");
    assert_eq!((base.cc, base.sloc), (10, 5));
}

#[test]
fn test_cc_increase_above_threshold_raises_e102() {
    let diagnostics = edited(11, 50, 12, 40);

    assert_eq!(codes(&diagnostics), vec![(CODE_COMPLEXITY_INCREASED, "m")]);
    let base = diagnostics[0].base.as_ref().expect("E102 has a base");
    assert_eq!(base.path, path("A.java"));
    assert_eq!((base.cc, base.sloc), (11, 50));
}

#[test]
fn test_base_sloc_9_plus_1_passes() {
    assert!(edited(11, 9, 11, 10).is_empty());
}

#[test]
fn test_base_sloc_9_plus_2_fails() {
    assert_eq!(
        codes(&edited(11, 9, 11, 11)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
}

#[test]
fn test_base_sloc_100_plus_10_passes_plus_11_fails() {
    assert!(edited(11, 100, 11, 110).is_empty());
    assert_eq!(
        codes(&edited(11, 100, 11, 111)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
}

#[test]
fn test_base_sloc_0_plus_1_passes_plus_2_fails() {
    assert!(edited(11, 0, 11, 1).is_empty());
    assert_eq!(
        codes(&edited(11, 0, 11, 2)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
}

#[test]
fn test_deletion_never_fails() {
    let diagnostics = run(&[("A.java", vec![spec("m", "f", 20, 30)])], &[]);

    assert!(diagnostics.is_empty());
}

#[test]
fn test_improvement_never_fails() {
    assert!(edited(20, 30, 15, 20).is_empty());
}

#[test]
fn test_off_disables_the_code() {
    let mut e101_off = default_policy();
    e101_off.nsd_e101 = Severity::Off;
    let mut e102_off = default_policy();
    e102_off.nsd_e102 = Severity::Off;
    let crossing = (
        [("A.java", vec![spec("m", "before", 10, 5)])],
        [("A.java", vec![spec("m", "after", 11, 5)])],
    );
    let growing = (
        [("A.java", vec![spec("m", "before", 11, 5)])],
        [("A.java", vec![spec("m", "after", 12, 5)])],
    );

    assert!(run_with(&crossing.0, &crossing.1, &e101_off).is_empty());
    assert_eq!(
        codes(&run_with(&crossing.0, &crossing.1, &e102_off)),
        vec![(CODE_COMPLEXITY_ABOVE_THRESHOLD, "m")]
    );
    assert!(run_with(&growing.0, &growing.1, &e102_off).is_empty());
    assert_eq!(
        codes(&run_with(&growing.0, &growing.1, &e101_off)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
}

// ---------------------------------------------------------------------
// G102.
// ---------------------------------------------------------------------

#[test]
fn test_positional_fallback_that_could_hide_a_regression_raises_g102() {
    let diagnostics = run(
        &[(
            "A.java",
            vec![spec("cb", "x", 20, 10), spec("cb", "y", 12, 10)],
        )],
        &[("A.java", vec![spec("cb", "y2", 20, 10)])],
    );

    assert_eq!(codes(&diagnostics), vec![(CODE_MATCH_AMBIGUITY, "cb")]);
}

#[test]
fn test_one_to_one_positional_remainder_never_raises_g102() {
    assert!(edited(12, 10, 12, 10).is_empty());
    assert_eq!(
        codes(&edited(12, 10, 13, 10)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
}

fn moved_bucket(base_count: usize, candidate_count: usize, cc: u32) -> Vec<PolicyDiagnostic> {
    bucket_with(base_count, candidate_count, cc, &default_policy())
}

fn bucket_with(
    base_count: usize,
    candidate_count: usize,
    cc: u32,
    policy: &PolicyConfig,
) -> Vec<PolicyDiagnostic> {
    let specs = |prefix: &str, count: usize| -> Vec<Spec> {
        (0..count)
            .map(|k| spec(&format!("{prefix}{k}"), "same-body", cc, 10))
            .collect()
    };
    let base: Vec<(&str, Vec<Spec>)> = if base_count == 0 {
        Vec::new()
    } else {
        vec![("A.java", specs("old", base_count))]
    };
    let candidate: Vec<(&str, Vec<Spec>)> = if candidate_count == 0 {
        Vec::new()
    } else {
        vec![("B.java", specs("new", candidate_count))]
    };
    run_with(&base, &candidate, policy)
}

#[test]
fn test_tier3_bucket_that_could_change_a_verdict_raises_g102() {
    let diagnostics = moved_bucket(1, 2, 20);

    assert_eq!(
        codes(&diagnostics),
        vec![
            (CODE_MATCH_AMBIGUITY, "new0"),
            (CODE_MATCH_AMBIGUITY, "new1")
        ]
    );
}

#[test]
fn test_tier3_bucket_whose_pairings_agree_emits_the_common_verdict() {
    assert!(moved_bucket(2, 2, 20).is_empty());
}

#[test]
fn test_tier3_bucket_below_threshold_raises_nothing() {
    assert!(moved_bucket(2, 2, 5).is_empty());
    assert!(moved_bucket(1, 2, 5).is_empty());
}

#[test]
fn test_two_to_zero_bucket_raises_nothing() {
    assert!(moved_bucket(2, 0, 20).is_empty());
}

#[test]
fn test_zero_to_two_bucket_raises_plain_e101() {
    let diagnostics = moved_bucket(0, 2, 20);

    assert_eq!(
        codes(&diagnostics),
        vec![
            (CODE_COMPLEXITY_ABOVE_THRESHOLD, "new0"),
            (CODE_COMPLEXITY_ABOVE_THRESHOLD, "new1")
        ]
    );
}

#[test]
fn test_g102_requires_an_enabled_verdict() {
    let mut both_off = default_policy();
    both_off.nsd_e101 = Severity::Off;
    both_off.nsd_e102 = Severity::Off;
    let mut e101_off = default_policy();
    e101_off.nsd_e101 = Severity::Off;
    let mut e102_off = default_policy();
    e102_off.nsd_e102 = Severity::Off;

    assert!(bucket_with(1, 2, 20, &both_off).is_empty());
    assert!(bucket_with(1, 2, 20, &e101_off).is_empty());
    assert_eq!(bucket_with(1, 2, 20, &e102_off).len(), 2);
}

#[test]
fn test_exact_fingerprint_pair_uses_the_same_thresholds() {
    let exact = |base_cc: u32, base_sloc: usize, cc: u32, sloc: usize| {
        run(
            &[("A.java", vec![spec("m", "same", base_cc, base_sloc)])],
            &[("A.java", vec![spec("m", "same", cc, sloc)])],
        )
    };

    assert_eq!(
        codes(&exact(10, 5, 11, 5)),
        vec![(CODE_COMPLEXITY_ABOVE_THRESHOLD, "m")]
    );
    assert_eq!(
        codes(&exact(11, 50, 12, 40)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
    assert_eq!(
        codes(&exact(11, 9, 11, 11)),
        vec![(CODE_COMPLEXITY_INCREASED, "m")]
    );
    assert!(exact(11, 9, 11, 10).is_empty());
}

#[test]
fn test_off_applies_to_an_unmatched_callable() {
    let mut e101_off = default_policy();
    e101_off.nsd_e101 = Severity::Off;

    assert!(run_with(&[], &[("A.java", vec![spec("m", "f", 11, 5)])], &e101_off).is_empty());
}

#[test]
fn test_positional_remainder_with_a_surplus_candidate_raises_g102() {
    let diagnostics = run(
        &[("A.java", vec![spec("cb", "x", 20, 10)])],
        &[(
            "A.java",
            vec![spec("cb", "a", 20, 10), spec("cb", "b", 20, 10)],
        )],
    );

    assert_eq!(codes(&diagnostics)[0], (CODE_MATCH_AMBIGUITY, "cb"));
    assert_eq!(diagnostics[0].candidate_start_line, 1);
    assert_eq!(
        codes(&diagnostics),
        vec![(CODE_MATCH_AMBIGUITY, "cb"), (CODE_MATCH_AMBIGUITY, "cb")]
    );
    assert_eq!(diagnostics[1].candidate_start_line, 2);
}

#[test]
fn test_surplus_positional_candidate_cannot_hide_a_regression() {
    let mut e101_off = default_policy();
    e101_off.nsd_e101 = Severity::Off;
    let base = [("A.java", vec![spec("cb", "x", 12, 10)])];
    let scenario = |candidates: Vec<Spec>| {
        let diagnostics = run_with(&base, &[("A.java", candidates)], &e101_off);
        diagnostics
            .iter()
            .map(|d| (d.code, d.candidate_start_line))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        scenario(vec![spec("cb", "a", 12, 10), spec("cb", "b", 20, 10)]),
        vec![(CODE_MATCH_AMBIGUITY, 2)]
    );
    assert_eq!(
        scenario(vec![spec("cb", "b", 20, 10), spec("cb", "a", 12, 10)]),
        vec![(CODE_MATCH_AMBIGUITY, 1)]
    );
}

#[test]
fn test_g102_when_only_a_higher_cc_base_passes() {
    let diagnostics = run(
        &[(
            "A.java",
            vec![spec("cb", "low", 15, 10), spec("cb", "high", 20, 100)],
        )],
        &[("A.java", vec![spec("cb", "new", 15, 100)])],
    );

    assert_eq!(codes(&diagnostics), vec![(CODE_MATCH_AMBIGUITY, "cb")]);
}

#[test]
fn test_tier3_bucket_agreeing_verdict_pairs_members_in_order() {
    let diagnostics = run(
        &[(
            "A.java",
            vec![
                spec("old0", "same-body", 11, 10),
                spec("old1", "same-body", 11, 12),
            ],
        )],
        &[(
            "B.java",
            vec![
                spec("new0", "same-body", 12, 10),
                spec("new1", "same-body", 12, 12),
            ],
        )],
    );

    assert_eq!(
        codes(&diagnostics),
        vec![
            (CODE_COMPLEXITY_INCREASED, "new0"),
            (CODE_COMPLEXITY_INCREASED, "new1")
        ]
    );
    let base_lines: Vec<usize> = diagnostics
        .iter()
        .map(|d| d.base.as_ref().expect("paired").start_line)
        .collect();
    assert_eq!(base_lines, vec![1, 2]);
}

// ---------------------------------------------------------------------
// Scale and determinism.
// ---------------------------------------------------------------------

fn numbered(prefix: &str, fingerprint: &str, cc: u32) -> Vec<Spec> {
    (0..LARGE_GROUP)
        .map(|k| spec(&format!("{prefix}{k}"), fingerprint, cc, 10))
        .collect()
}

fn timed_classify(
    base: &[(&str, Vec<Spec>)],
    candidate: &[(&str, Vec<Spec>)],
) -> Vec<PolicyDiagnostic> {
    let (base_callables, base_metrics) = sides(base);
    let (candidate_callables, candidate_metrics) = sides(candidate);
    let matched = match_callables(&base_callables, &candidate_callables, &[]);
    let started = Instant::now();
    let diagnostics = classify(
        &base_metrics,
        &candidate_metrics,
        &matched,
        &default_policy(),
    );
    assert!(started.elapsed() < BOUND, "classify exceeded {BOUND:?}");
    diagnostics
}

#[test]
fn test_large_same_key_group_classifies_in_bounded_time() {
    let same_key = |cc: u32, fingerprint_prefix: &str| -> Vec<Spec> {
        (0..LARGE_GROUP)
            .map(|k| spec("cb", &format!("{fingerprint_prefix}-{k}"), cc, 10))
            .collect()
    };

    let diagnostics = timed_classify(
        &[("A.java", same_key(11, "b"))],
        &[("A.java", same_key(12, "c"))],
    );

    assert_eq!(diagnostics.len(), LARGE_GROUP);
    assert!(diagnostics
        .iter()
        .all(|d| d.code == CODE_COMPLEXITY_INCREASED));
}

#[test]
fn test_large_tier3_bucket_classifies_in_bounded_time() {
    let diagnostics = timed_classify(
        &[("A.java", numbered("old", "shared", 11))],
        &[("B.java", numbered("new", "shared", 12))],
    );

    assert_eq!(diagnostics.len(), LARGE_GROUP);
    assert!(diagnostics
        .iter()
        .all(|d| d.code == CODE_COMPLEXITY_INCREASED));
}

#[test]
fn test_output_is_deterministic_under_input_order() {
    let base = vec![
        ("A.java", vec![spec("grow", "g1", 11, 10)]),
        (
            "B.java",
            vec![spec("cb", "x", 20, 10), spec("cb", "y", 12, 10)],
        ),
        ("C.java", vec![spec("moved", "mv", 20, 10)]),
    ];
    let candidate = vec![
        ("A.java", vec![spec("grow", "g2", 12, 10)]),
        ("B.java", vec![spec("cb", "y2", 20, 10)]),
        (
            "D.java",
            vec![spec("moved", "mv", 20, 10), spec("fresh", "n", 11, 3)],
        ),
        (
            "E.java",
            vec![spec("one", "o1", 20, 10), spec("two", "o1", 20, 10)],
        ),
    ];
    let forward = run(&base, &candidate);

    let mut base_reversed = base;
    base_reversed.reverse();
    let mut candidate_reversed = candidate;
    candidate_reversed.reverse();
    let reversed = run(&base_reversed, &candidate_reversed);

    assert_eq!(forward, reversed);
    assert_eq!(
        codes(&forward),
        vec![
            (CODE_COMPLEXITY_INCREASED, "grow"),
            (CODE_MATCH_AMBIGUITY, "cb"),
            (CODE_COMPLEXITY_ABOVE_THRESHOLD, "fresh"),
            (CODE_COMPLEXITY_ABOVE_THRESHOLD, "one"),
            (CODE_COMPLEXITY_ABOVE_THRESHOLD, "two"),
        ]
    );
}

// ---------------------------------------------------------------------
// Real commits.
// ---------------------------------------------------------------------

fn java_method(name: &str, ifs: usize) -> String {
    let mut body = String::new();
    for k in 0..ifs {
        body.push_str(&format!("        if (x > {k}) {{ x++; }}\n"));
    }
    format!("    int {name}(int x) {{\n{body}        return x;\n    }}\n")
}

fn java_class(class: &str, methods: &[String]) -> Vec<u8> {
    format!("class {class} {{\n{}}}\n", methods.concat()).into_bytes()
}

fn commit(repo: &Repository, files: &[(&str, Vec<u8>)]) -> Oid {
    let entries: Vec<_> = files
        .iter()
        .map(|(file, bytes)| (file.as_bytes().to_vec(), MODE_REGULAR, bytes.clone()))
        .collect();
    common::commit_entries(repo, &entries)
}

fn analyzed(
    snapshot: &CommitSnapshot,
    repo: &Repository,
    wanted: &BTreeSet<RepoPath>,
) -> (Vec<FileCallables>, Vec<FileMetrics>) {
    let mut callables = Vec::new();
    let mut metrics = Vec::new();
    for entry in snapshot.entries.iter().filter(|e| wanted.contains(&e.path)) {
        let Some(bytes) = snapshot.read(repo, entry).expect("read blob") else {
            continue;
        };
        let Ok(analysis) = analyze_file(Path::new(&entry.path.render()), &bytes) else {
            continue;
        };
        callables.push(FileCallables {
            path: entry.path.clone(),
            callables: analysis
                .callables
                .iter()
                .map(|c| (c.identity.clone(), c.body_fingerprint.clone()))
                .collect(),
        });
        metrics.push(FileMetrics {
            path: entry.path.clone(),
            callables: analysis
                .callables
                .iter()
                .map(|c| c.metrics.clone())
                .collect(),
        });
    }
    (callables, metrics)
}

fn evaluate(
    repo: &Repository,
    base: Oid,
    candidate: Oid,
    policy: &PolicyConfig,
) -> Vec<PolicyDiagnostic> {
    let changes = diff_commit_to_commit(repo, Some(base), candidate).expect("diff commits");
    let mut base_paths = BTreeSet::new();
    let mut candidate_paths = BTreeSet::new();
    for change in &changes {
        match change {
            Change::Added { path, .. } => {
                candidate_paths.insert(path.clone());
            }
            Change::Deleted { path, .. } => {
                base_paths.insert(path.clone());
            }
            Change::Modified { path, .. } | Change::Typechange { path, .. } => {
                base_paths.insert(path.clone());
                candidate_paths.insert(path.clone());
            }
            Change::Renamed { from, to, .. } => {
                base_paths.insert(from.clone());
                candidate_paths.insert(to.clone());
            }
        }
    }
    let base_snapshot = CommitSnapshot::at(repo, base).expect("base snapshot");
    let candidate_snapshot = CommitSnapshot::at(repo, candidate).expect("candidate snapshot");
    let (base_callables, base_metrics) = analyzed(&base_snapshot, repo, &base_paths);
    let (candidate_callables, candidate_metrics) =
        analyzed(&candidate_snapshot, repo, &candidate_paths);
    let matched = match_callables(&base_callables, &candidate_callables, &changes);
    classify(&base_metrics, &candidate_metrics, &matched, policy)
}

fn scratch_cc3_to_cc11(policy: &PolicyConfig) -> Vec<PolicyDiagnostic> {
    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[(
            "Widget.java",
            java_class("Widget", &[java_method("run", 2)]),
        )],
    );
    let candidate = commit(
        &repo,
        &[(
            "Widget.java",
            java_class("Widget", &[java_method("run", 10)]),
        )],
    );
    evaluate(&repo, base, candidate, policy)
}

#[test]
fn test_scratch_repo_new_complex_method_raises_e101() {
    let diagnostics = scratch_cc3_to_cc11(&default_policy());

    assert_eq!(
        codes(&diagnostics),
        vec![(CODE_COMPLEXITY_ABOVE_THRESHOLD, "run")]
    );
    assert_eq!(diagnostics[0].candidate_path, path("Widget.java"));
    let base = diagnostics[0].base.as_ref().expect("the method was paired");
    assert_eq!(base.cc, 3);

    let mut off = default_policy();
    off.nsd_e101 = Severity::Off;
    assert!(scratch_cc3_to_cc11(&off).is_empty());
}

#[test]
fn test_line_only_move_and_rename_and_cross_file_move_raise_nothing() {
    let policy = default_policy();
    let big = || java_method("big", 19);

    let (_dir, repo) = common::init_repo();
    let base = commit(&repo, &[("A.java", java_class("A", &[big()]))]);
    let shifted = format!(
        "\n\n// moved down\n\n{}",
        String::from_utf8(java_class("A", &[big()])).unwrap()
    );
    let candidate = commit(&repo, &[("A.java", shifted.into_bytes())]);
    assert!(evaluate(&repo, base, candidate, &policy).is_empty());

    let (_dir, repo) = common::init_repo();
    let padding: Vec<String> = (0..10)
        .map(|k| java_method(&format!("pad{k}"), 1))
        .collect();
    let mut methods = padding.clone();
    methods.push(big());
    let base = commit(&repo, &[("Old.java", java_class("Widget", &methods))]);
    let candidate = commit(&repo, &[("New.java", java_class("Widget", &methods))]);
    let renamed = diff_commit_to_commit(&repo, Some(base), candidate).expect("diff");
    assert!(
        matches!(renamed.as_slice(), [Change::Renamed { .. }]),
        "{renamed:?}"
    );
    assert!(evaluate(&repo, base, candidate, &policy).is_empty());

    let (_dir, repo) = common::init_repo();
    let base = commit(
        &repo,
        &[
            ("A.java", java_class("A", &[java_method("other", 1), big()])),
            ("B.java", java_class("B", &[java_method("keep", 1)])),
        ],
    );
    let candidate = commit(
        &repo,
        &[
            ("A.java", java_class("A", &[java_method("other", 1)])),
            ("B.java", java_class("B", &[java_method("keep", 1), big()])),
        ],
    );
    assert!(evaluate(&repo, base, candidate, &policy).is_empty());
}

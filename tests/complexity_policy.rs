mod common;

use git2::Repository;
use nsd::check::{run_check, CheckDiagnostic, CheckMode, CheckRequest};
use nsd::config::{Config, CODE_INVALID_CONFIG};

const MODE_REGULAR: i32 = 0o100644;
const DEFAULT_POLICY: &str = "version: 1\npolicy:\n  NSD-V101: off\n  NSD-V102: off\n";
const CC_ONLY_POLICY: &str =
    "version: 1\ncomplexity:\n  sloc_growth: false\npolicy:\n  NSD-V101: off\n  NSD-V102: off\n";
const HIGH_COMPLEXITY_BRANCHES: usize = 11;
const SLOC_EXTENSION: usize = 10;

fn source(branches: usize, extra_lines: usize) -> Vec<u8> {
    let mut source = String::from("export function evaluate(value: number) {\nlet result = 0;\n");
    for index in 0..branches {
        source.push_str(&format!("if (value > {index}) {{ result += {index}; }}\n"));
    }
    for index in 0..extra_lines {
        source.push_str(&format!("result += value + {index};\n"));
    }
    source.push_str("return result;\n}\n");
    source.into_bytes()
}

fn commit(repo: &Repository, config: &str, source: Vec<u8>) -> git2::Oid {
    common::commit_entries(
        repo,
        &[
            (
                b"nsd.yml".to_vec(),
                MODE_REGULAR,
                config.as_bytes().to_vec(),
            ),
            (b"src/evaluate.ts".to_vec(), MODE_REGULAR, source),
        ],
    )
}

#[test]
fn sloc_only_growth_can_be_disabled_without_disabling_cc_growth() {
    let (directory, repo) = common::init_repo();
    let base = commit(&repo, CC_ONLY_POLICY, source(HIGH_COMPLEXITY_BRANCHES, 0));
    commit(
        &repo,
        CC_ONLY_POLICY,
        source(HIGH_COMPLEXITY_BRANCHES, SLOC_EXTENSION),
    );
    let check = |reference: String| {
        run_check(&CheckRequest {
            repository: directory.path(),
            mode: CheckMode::Base {
                reference,
                worktree: false,
            },
            config_path: None,
            allow_new_suppressions: false,
        })
    };
    let outcome = check(base.to_string());
    assert_eq!(outcome.exit_status, 0, "{:?}", outcome.diagnostics);
    assert!(outcome.diagnostics.is_empty());

    commit(
        &repo,
        CC_ONLY_POLICY,
        source(HIGH_COMPLEXITY_BRANCHES + 1, SLOC_EXTENSION),
    );
    let outcome = check(base.to_string());
    assert_eq!(outcome.exit_status, 1);
    let complexities: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            CheckDiagnostic::Complexity(found) => Some(found),
            _ => None,
        })
        .collect();
    assert_eq!(complexities.len(), 1);
    assert_eq!(complexities[0].code, "NSD-E102");
}

#[test]
fn candidate_cannot_disable_its_own_sloc_regression() {
    let (directory, repo) = common::init_repo();
    let base = commit(&repo, DEFAULT_POLICY, source(HIGH_COMPLEXITY_BRANCHES, 0));
    commit(
        &repo,
        CC_ONLY_POLICY,
        source(HIGH_COMPLEXITY_BRANCHES, SLOC_EXTENSION),
    );
    let outcome = run_check(&CheckRequest {
        repository: directory.path(),
        mode: CheckMode::Base {
            reference: base.to_string(),
            worktree: false,
        },
        config_path: None,
        allow_new_suppressions: false,
    });
    assert_eq!(outcome.exit_status, 1);
    assert!(outcome.policy.sloc_growth);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code() == "NSD-E102"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code() == "NSD-C101"));
}

#[test]
fn sloc_growth_defaults_and_fingerprints_are_explicit() {
    let defaults = Config::parse(DEFAULT_POLICY.as_bytes()).expect("default config");
    let disabled = Config::parse(CC_ONLY_POLICY.as_bytes()).expect("CC-only config");
    let enabled = Config::parse(
        b"version: 1\ncomplexity: {sloc_growth: true}\npolicy: {NSD-V101: off, NSD-V102: off}\n",
    )
    .expect("explicit default");
    assert!(defaults.policy.sloc_growth);
    assert!(!disabled.policy.sloc_growth);
    assert_eq!(defaults.fingerprint(), enabled.fingerprint());
    assert_ne!(defaults.fingerprint(), disabled.fingerprint());
    assert_eq!(defaults.measurement, disabled.measurement);
}

#[test]
fn invalid_sloc_policy_is_refused() {
    for yaml in [
        "complexity: null",
        "complexity: !!str",
        "complexity: {sloc_growth: null}",
        "complexity: {sloc_growth: deny}",
        "complexity: {sloc_growth: 0}",
        "complexity: {sloc_growth: false, sloc_growth: true}",
        "complexity: {unknown: false}",
    ] {
        let error = Config::parse(format!("version: 1\n{yaml}\n").as_bytes()).expect_err(yaml);
        assert_eq!(error.code(), CODE_INVALID_CONFIG, "{yaml}");
    }
}

//! WS-4: `scripts/self_check_base.sh` resolves the `--base` argument of the
//! repository's own CI smoke run. Each base is fed to a real `nsd check` over
//! a scratch repository, so a base that compares nothing (the pushed HEAD
//! with itself) or resolves to nothing fails here, not in CI.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use git2::{Oid, Repository};
use serde_json::Value;

const MODE_REGULAR: i32 = 0o100644;
const ALL_ZERO_SHA: &str = "0000000000000000000000000000000000000000";
const UNKNOWN_SHA: &str = "1234567890abcdef1234567890abcdef12345678";
const CHANGED_CALLABLE: &str = "f";
const BASE_JAVA: &str = "class A {\n    int f(int x) {\n        return x;\n    }\n}\n";
const EDITED_JAVA: &str =
    "class A {\n    int f(int x) {\n        int y = x + 1;\n        return y;\n    }\n}\n";
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/self_check_base.sh");

fn commit_java(repo: &Repository, text: &str) -> Oid {
    common::commit_entries(
        repo,
        &[(b"A.java".to_vec(), MODE_REGULAR, text.as_bytes().to_vec())],
    )
}

fn run_script(repo_dir: &Path, event: &str, before: &str, base_ref: &str) -> Output {
    Command::new("bash")
        .arg(SCRIPT)
        .args([event, before, base_ref])
        .current_dir(repo_dir)
        .output()
        .expect("run the base script")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn changed_names(repo_dir: &Path, base: &str) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_nsd"))
        .args(["check", "--base", base, "--format", "json"])
        .current_dir(repo_dir)
        .output()
        .expect("run nsd check");
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "check --base {base} printed no JSON ({error}): {}",
            stderr_of(&output)
        )
    });
    parsed["entities"]
        .as_array()
        .expect("an entities array")
        .iter()
        .map(|entity| entity["name"].as_str().expect("entity name").to_string())
        .collect()
}

fn diff_is_not_empty(repo: &Repository, base: &str) -> bool {
    let base_commit = repo
        .revparse_single(base)
        .and_then(|object| object.peel_to_commit())
        .expect("resolve the base");
    let head = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .expect("resolve HEAD");
    let merge_base = repo
        .merge_base(base_commit.id(), head.id())
        .expect("find the merge base");
    let old_tree = repo
        .find_commit(merge_base)
        .and_then(|commit| commit.tree())
        .expect("merge base tree");
    let new_tree = head.tree().expect("HEAD tree");
    let diff = repo
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
        .expect("diff the trees");
    diff.deltas().len() > 0
}

fn assert_change_set_names_the_edit(dir: &Path, repo: &Repository, base: &str) {
    assert!(diff_is_not_empty(repo, base), "empty diff for base {base}");
    assert_eq!(
        changed_names(dir, base),
        [CHANGED_CALLABLE],
        "base {base} did not yield the edited callable"
    );
}

fn two_commit_repo() -> (tempfile::TempDir, Repository, Oid) {
    let (dir, repo) = common::init_repo();
    let first = commit_java(&repo, BASE_JAVA);
    commit_java(&repo, EDITED_JAVA);
    (dir, repo, first)
}

#[cfg(unix)]
#[test]
fn test_push_base_is_the_pre_push_sha_and_the_change_set_is_not_empty() {
    let (dir, repo, previous_tip) = two_commit_repo();

    let output = run_script(dir.path(), "push", &previous_tip.to_string(), "");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let base = stdout_of(&output);
    assert_eq!(base, previous_tip.to_string());
    assert_change_set_names_the_edit(dir.path(), &repo, &base);
}

#[cfg(unix)]
#[test]
fn test_pull_request_base_is_the_target_branch_and_the_change_set_is_not_empty() {
    let (dir, repo, target_tip) = two_commit_repo();
    repo.reference(
        "refs/remotes/origin/main",
        target_tip,
        true,
        "target branch",
    )
    .expect("create the remote-tracking ref");

    let output = run_script(dir.path(), "pull_request", "", "main");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let base = stdout_of(&output);
    assert_eq!(base, "origin/main");
    assert_change_set_names_the_edit(dir.path(), &repo, &base);
}

#[cfg(unix)]
#[test]
fn test_all_zero_before_falls_back_to_the_parent_commit() {
    let (dir, repo, _) = two_commit_repo();

    let output = run_script(dir.path(), "push", ALL_ZERO_SHA, "");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let base = stdout_of(&output);
    assert_ne!(base, ALL_ZERO_SHA);
    assert_change_set_names_the_edit(dir.path(), &repo, &base);
}

#[cfg(unix)]
#[test]
fn test_unknown_before_falls_back_to_the_parent_commit() {
    let (dir, repo, _) = two_commit_repo();

    let output = run_script(dir.path(), "push", UNKNOWN_SHA, "");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let base = stdout_of(&output);
    assert_ne!(base, UNKNOWN_SHA);
    assert_change_set_names_the_edit(dir.path(), &repo, &base);
}

#[cfg(unix)]
#[test]
fn test_root_commit_push_prints_no_base_and_a_notice() {
    let (dir, repo) = common::init_repo();
    commit_java(&repo, BASE_JAVA);

    let output = run_script(dir.path(), "push", ALL_ZERO_SHA, "");

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "");
    assert!(
        stderr_of(&output).contains("notice:"),
        "{}",
        stderr_of(&output)
    );
}

#[cfg(unix)]
#[test]
fn test_pull_request_without_a_base_ref_is_an_error() {
    let (dir, _repo, _) = two_commit_repo();

    let output = run_script(dir.path(), "pull_request", "", "");

    assert!(!output.status.success());
    assert_eq!(stdout_of(&output), "");
    assert!(
        stderr_of(&output).contains("error:"),
        "{}",
        stderr_of(&output)
    );
}

#[cfg(unix)]
#[test]
fn test_other_events_are_an_error() {
    let (dir, _repo, tip) = two_commit_repo();

    let output = run_script(dir.path(), "workflow_dispatch", &tip.to_string(), "main");

    assert!(!output.status.success());
    assert_eq!(stdout_of(&output), "");
    assert!(
        stderr_of(&output).contains("error:"),
        "{}",
        stderr_of(&output)
    );
}

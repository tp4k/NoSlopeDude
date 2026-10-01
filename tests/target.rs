//! Revision probing of a local target (D6) must not run code configured by
//! the scanned checkout's own `.git/config`.

use std::fs;
use std::path::Path;

use git2::{Repository, Signature};
use nsd::target::{classify, resolve};
use tempfile::TempDir;

const TRACKED_FILE: &str = "Tracked.java";
const UNTRACKED_FILE: &str = "Untracked.java";
const HOOK_FILE: &str = "hook.sh";
const MARKER_FILE: &str = "hook-ran.marker";
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// A repository with one committed, unmodified file; returns the commit sha.
fn committed_repo() -> (TempDir, Repository, String) {
    let dir = TempDir::new().expect("create a temp dir for the repository");
    let repo = Repository::init(dir.path()).expect("init the repository");
    fs::write(dir.path().join(TRACKED_FILE), "class Tracked {}\n").expect("write a tracked file");
    let mut index = repo.index().expect("open the index");
    index
        .add_path(Path::new(TRACKED_FILE))
        .expect("stage the tracked file");
    index.write().expect("write the index");
    let tree = repo
        .find_tree(index.write_tree().expect("write the tree"))
        .expect("find the tree");
    let signature = Signature::now("nsd test", "test@example.invalid").expect("signature");
    let sha = repo
        .commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
        .expect("commit")
        .to_string();
    drop(tree);
    (dir, repo, sha)
}

fn resolved_revision(dir: &Path) -> nsd::model::Revision {
    let target = classify(dir.to_str().expect("utf-8 temp path"));
    resolve(&target).expect("resolve the local target").revision
}

#[cfg(unix)]
#[test]
fn test_resolving_an_untrusted_checkout_does_not_run_its_fsmonitor_hook() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, repo, sha) = committed_repo();
    let outside = TempDir::new().expect("create a temp dir for the hook");
    let marker = outside.path().join(MARKER_FILE);
    let hook = outside.path().join(HOOK_FILE);
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("write the hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(EXECUTABLE_MODE))
        .expect("make the hook executable");
    repo.config()
        .expect("open the repository config")
        .set_str("core.fsmonitor", hook.to_str().expect("utf-8 hook path"))
        .expect("set core.fsmonitor");

    let revision = resolved_revision(dir.path());

    assert!(!marker.exists(), "the checkout's fsmonitor hook ran");
    assert_eq!(revision.sha, Some(sha));
    assert_eq!(revision.dirty, Some(false));
}

#[test]
fn test_local_revision_still_reports_sha_and_dirty() {
    let (dir, _repo, sha) = committed_repo();

    let clean = resolved_revision(dir.path());
    fs::write(dir.path().join(UNTRACKED_FILE), "class Untracked {}\n").expect("write a new file");
    let dirty = resolved_revision(dir.path());

    assert_eq!((clean.sha, clean.dirty), (Some(sha.clone()), Some(false)));
    assert_eq!((dirty.sha, dirty.dirty), (Some(sha), Some(true)));
}

//! Revision probing of a local target (D6) must not run code configured by
//! the scanned checkout's own `.git/config`.

mod common;

use std::fs;
use std::path::Path;

use git2::Repository;
use nsd::target::{classify, resolve};
use tempfile::TempDir;

const MODE_REGULAR: i32 = 0o100644;
const TRACKED_FILE: &str = "Tracked.java";
const UNTRACKED_FILE: &str = "Untracked.java";
const HOOK_FILE: &str = "hook.sh";
const MARKER_FILE: &str = "hook-ran.marker";
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// A repository with one committed, unmodified file; returns the commit sha.
fn committed_repo() -> (TempDir, Repository, String) {
    let (dir, repo) = common::init_repo();
    let content = b"class Tracked {}\n";
    let commit = common::commit_entries(
        &repo,
        &[(
            TRACKED_FILE.as_bytes().to_vec(),
            MODE_REGULAR,
            content.to_vec(),
        )],
    );
    fs::write(dir.path().join(TRACKED_FILE), content).expect("write a tracked file");
    let mut index = repo.index().expect("open the index");
    index
        .add_path(Path::new(TRACKED_FILE))
        .expect("stage the tracked file");
    index.write().expect("write the index");
    (dir, repo, commit.to_string())
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
    assert_eq!(revision.dirty, None);
}

#[test]
fn test_local_revision_reports_sha_and_no_dirty_flag() {
    let (dir, _repo, sha) = committed_repo();

    let clean = resolved_revision(dir.path());
    fs::write(dir.path().join(UNTRACKED_FILE), "class Untracked {}\n").expect("write a new file");
    let dirty = resolved_revision(dir.path());

    assert_eq!((clean.sha, clean.dirty), (Some(sha.clone()), None));
    assert_eq!((dirty.sha, dirty.dirty), (Some(sha), None));
}

const HOOKS_DIR: &str = "hooks";
const POST_INDEX_CHANGE_HOOK: &str = "post-index-change";
const FETCH_MARKER_FILE: &str = "fetch-ran.marker";
const RENAMED_FILE: &str = "Renamed.java";
const FUTURE_MTIME_SECS: u64 = 3600;

fn bump_mtime(path: &Path) {
    let file = fs::File::options()
        .write(true)
        .open(path)
        .expect("open the file to bump its mtime");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(FUTURE_MTIME_SECS);
    file.set_modified(later).expect("bump the mtime");
}

#[cfg(unix)]
#[test]
fn test_resolving_an_untrusted_checkout_does_not_run_its_post_index_change_hook() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, repo, sha) = committed_repo();
    let outside = TempDir::new().expect("create a temp dir for the marker");
    let marker = outside.path().join(MARKER_FILE);
    let hooks = repo.path().join(HOOKS_DIR);
    fs::create_dir_all(&hooks).expect("create the hooks directory");
    let hook = hooks.join(POST_INDEX_CHANGE_HOOK);
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("write the hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(EXECUTABLE_MODE))
        .expect("make the hook executable");
    bump_mtime(&dir.path().join(TRACKED_FILE));

    let revision = resolved_revision(dir.path());

    assert!(
        !marker.exists(),
        "the checkout's post-index-change hook ran"
    );
    assert_eq!(revision.sha, Some(sha));
}

#[cfg(unix)]
#[test]
fn test_resolving_a_partial_clone_checkout_does_not_run_its_transport() {
    let (dir, repo, sha) = committed_repo();
    let outside = TempDir::new().expect("create a temp dir for the marker");
    let marker = outside.path().join(FETCH_MARKER_FILE);

    let tracked = dir.path().join(TRACKED_FILE);
    let blob = repo
        .head()
        .and_then(|head| head.peel_to_tree())
        .and_then(|tree| tree.get_path(Path::new(TRACKED_FILE)))
        .expect("find the committed blob")
        .id()
        .to_string();
    let mut content = fs::read_to_string(&tracked).expect("read the tracked file");
    content.push_str("// renamed and edited\n");
    fs::write(dir.path().join(RENAMED_FILE), content).expect("write the renamed file");
    fs::remove_file(&tracked).expect("delete the original file");
    let mut index = repo.index().expect("open the index");
    index
        .remove_path(Path::new(TRACKED_FILE))
        .expect("unstage the original");
    index
        .add_path(Path::new(RENAMED_FILE))
        .expect("stage the renamed file");
    index.write().expect("write the index");
    let (fan_out, rest) = blob.split_at(2);
    fs::remove_file(repo.path().join("objects").join(fan_out).join(rest))
        .expect("delete the original blob object");

    let mut config = repo.config().expect("open the repository config");
    config
        .set_i32("core.repositoryformatversion", 1)
        .expect("enable extensions");
    config
        .set_str("extensions.partialClone", "origin")
        .expect("set the promisor remote");
    config
        .set_bool("remote.origin.promisor", true)
        .expect("mark the remote as a promisor");
    config
        .set_str(
            "remote.origin.url",
            &format!("ext::sh -c touch% {}", marker.display()),
        )
        .expect("set the transport");
    config
        .set_str("protocol.ext.allow", "always")
        .expect("allow the ext transport");

    let revision = resolved_revision(dir.path());

    assert!(!marker.exists(), "the checkout's transport ran");
    assert_eq!(revision.sha, Some(sha));
}

const FILTER_MARKER_FILE: &str = "filter-ran.marker";
const GITATTRIBUTES_FILE: &str = ".gitattributes";

#[cfg(unix)]
#[test]
fn test_resolving_an_untrusted_checkout_does_not_run_its_clean_filter() {
    let (dir, repo, sha) = committed_repo();
    let outside = TempDir::new().expect("create a temp dir for the marker");
    let marker = outside.path().join(FILTER_MARKER_FILE);
    fs::write(dir.path().join(GITATTRIBUTES_FILE), "* filter=x\n").expect("write gitattributes");
    repo.config()
        .expect("open the repository config")
        .set_str(
            "filter.x.clean",
            &format!("touch '{}'; cat", marker.display()),
        )
        .expect("set filter.x.clean");
    bump_mtime(&dir.path().join(TRACKED_FILE));

    let revision = resolved_revision(dir.path());

    assert!(!marker.exists(), "the checkout's clean filter ran");
    assert_eq!(revision.sha, Some(sha));
    assert_eq!(revision.dirty, None);
}

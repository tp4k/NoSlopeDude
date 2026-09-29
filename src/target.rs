//! Resolves a CLI target string into a scanned root plus its revision.
//!
//! Local folders are canonicalized in place; a public GitHub URL is
//! shallow-cloned by the system `git` binary (D5) into a temporary
//! directory that is kept alive for the lifetime of the resolved target.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};

use crate::model::{RemoteTarget, Revision, Target};

const GITHUB_URL_PREFIXES: [&str; 2] = ["https://github.com/", "http://github.com/"];

/// A target resolved to a concrete, readable root on disk.
pub struct ResolvedTarget {
    pub root: PathBuf,
    pub revision: Revision,
    /// Keeps a remote target's shallow-clone directory alive for as long as
    /// the resolved target is in scope. `None` for a local target.
    _clone_dir: Option<tempfile::TempDir>,
}

/// Classifies a raw CLI argument as a local path or a public GitHub URL.
pub fn classify(input: &str) -> Target {
    if is_github_url(input) {
        Target::Remote(RemoteTarget {
            url: input.to_string(),
        })
    } else {
        Target::Local(PathBuf::from(input))
    }
}

/// Resolves a classified target to a scanned root and its revision.
///
/// An unreadable local target or a failed clone is fatal (D18).
pub fn resolve(target: &Target) -> anyhow::Result<ResolvedTarget> {
    match target {
        Target::Local(path) => resolve_local(path),
        Target::Remote(remote) => resolve_remote(remote),
    }
}

fn is_github_url(input: &str) -> bool {
    GITHUB_URL_PREFIXES
        .iter()
        .any(|prefix| input.starts_with(prefix))
}

fn resolve_local(path: &Path) -> anyhow::Result<ResolvedTarget> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot read local target {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("local target {} is not a directory", canonical.display());
    }
    let revision = local_git_revision(&canonical);
    Ok(ResolvedTarget {
        root: canonical,
        revision,
        _clone_dir: None,
    })
}

fn resolve_remote(remote: &RemoteTarget) -> anyhow::Result<ResolvedTarget> {
    let clone_dir = tempfile::TempDir::new()
        .context("cannot create a temporary directory for the shallow clone")?;
    let clone_dir_str = clone_dir
        .path()
        .to_str()
        .context("temporary clone directory path is not valid UTF-8")?;
    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--single-branch",
            &remote.url,
            clone_dir_str,
        ])
        .status()
        .with_context(|| format!("failed to run git clone for {}", remote.url))?;
    if !status.success() {
        bail!("git clone failed for {}", remote.url);
    }
    let sha = git_head_sha(clone_dir.path());
    let revision = Revision {
        sha,
        dirty: Some(false),
        unavailable_reason: None,
    };
    let root = clone_dir.path().to_path_buf();
    Ok(ResolvedTarget {
        root,
        revision,
        _clone_dir: Some(clone_dir),
    })
}

fn local_git_revision(root: &Path) -> Revision {
    let is_work_tree = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim() == "true")
        .unwrap_or(false);

    if !is_work_tree {
        return Revision {
            sha: None,
            dirty: None,
            unavailable_reason: Some("not_a_git_repository".to_string()),
        };
    }

    let sha = git_head_sha(root);
    let dirty = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());

    Revision {
        sha,
        dirty,
        unavailable_reason: None,
    }
}

fn git_head_sha(root: &Path) -> Option<String> {
    Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

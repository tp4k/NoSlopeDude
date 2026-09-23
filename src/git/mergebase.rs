//! Merge-base resolution with an actionable `NSD-G101` on a shallow clone,
//! an unresolvable ref, an unborn `HEAD`, or unrelated histories.

use git2::{ErrorCode, Oid, Repository};

use super::{wrap_git_error, GitError, CODE_SNAPSHOT_UNAVAILABLE};

/// Resolves `merge_base(HEAD, other_ref)` in `repo`.
///
/// Fails with `NSD-G101` (D21) when: `HEAD` is unborn, `other_ref` cannot be
/// resolved to a commit, or no common ancestor exists — either because the
/// histories are unrelated or because the merge base lies beyond a shallow
/// clone's history boundary (in which case the message names the shallow
/// clone and how to deepen it).
pub fn merge_base(repo: &Repository, other_ref: &str) -> Result<Oid, GitError> {
    let head_commit = repo
        .head()
        .map_err(|err| head_error(&err))?
        .peel_to_commit()
        .map_err(|err| wrap_git_error("HEAD does not resolve to a commit", &err))?;

    let other_commit = repo
        .revparse_single(other_ref)
        .and_then(|object| object.peel_to_commit())
        .map_err(|err| {
            GitError::new(
                CODE_SNAPSHOT_UNAVAILABLE,
                format!("cannot resolve ref '{other_ref}' to a commit: {err}"),
            )
        })?;

    repo.merge_base(head_commit.id(), other_commit.id())
        .map_err(|err| no_merge_base_error(repo, other_ref, &err))
}

fn head_error(err: &git2::Error) -> GitError {
    if err.code() == ErrorCode::UnbornBranch {
        return GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            "HEAD is unborn (no commits yet); merge-base has no base commit to start from",
        );
    }
    wrap_git_error("cannot resolve HEAD", err)
}

fn no_merge_base_error(repo: &Repository, other_ref: &str, err: &git2::Error) -> GitError {
    if err.code() == ErrorCode::NotFound && repo.is_shallow() {
        return GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!(
                "no merge base between HEAD and '{other_ref}': this is a shallow clone and the \
                 merge base lies beyond its history boundary; deepen it (e.g. `git fetch \
                 --unshallow`) and retry"
            ),
        );
    }
    if err.code() == ErrorCode::NotFound {
        return GitError::new(
            CODE_SNAPSHOT_UNAVAILABLE,
            format!("no merge base between HEAD and '{other_ref}': histories are unrelated"),
        );
    }
    wrap_git_error(
        &format!("cannot resolve merge base between HEAD and '{other_ref}'"),
        err,
    )
}

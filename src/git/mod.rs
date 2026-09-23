//! Git-backed read primitives (WS-1): repository-relative byte paths
//! (`path`), `Commit`/`Index`/`Worktree` snapshots (`snapshot`), and
//! merge-base resolution (`mergebase`). WS-2 and WS-4 add their own
//! `pub mod` lines here for `diff` and `discovery`.

pub mod diff;
pub mod discovery;
pub mod mergebase;
pub mod path;
pub mod snapshot;

/// The one diagnostic code this workstream raises: "merge base or required
/// snapshot unavailable" (`nsd-plan-final.md` *Diagnostics*). The full
/// twelve-code diagnostics enum is M3's `src/policy/` and is not created
/// here (D21) — until then, every fallible Git read in this module reports
/// through this single stable code.
pub const CODE_SNAPSHOT_UNAVAILABLE: &str = "NSD-G101";

/// A Git-domain error carrying a stable diagnostic code (D21).
#[derive(Debug)]
pub struct GitError {
    code: &'static str,
    message: String,
}

impl GitError {
    pub fn new(code: &'static str, message: impl Into<String>) -> GitError {
        GitError {
            code,
            message: message.into(),
        }
    }

    /// The stable diagnostic code, e.g. `"NSD-G101"`.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for GitError {}

/// Wraps a `git2::Error` encountered while building a snapshot or resolving
/// a merge base as a `GitError` under `CODE_SNAPSHOT_UNAVAILABLE`, prefixing
/// it with `context` for a message a user can act on.
pub(crate) fn wrap_git_error(context: &str, err: &git2::Error) -> GitError {
    GitError::new(CODE_SNAPSHOT_UNAVAILABLE, format!("{context}: {err}"))
}

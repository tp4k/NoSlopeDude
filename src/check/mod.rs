//! M5-4: one uncached `check` run over snapshots (`nsd-plan-final.md` *CLI
//! and configuration*). Every internal failure is a diagnostic in the
//! outcome, never an `Err`, so the exit status is always 2-class for them.

use std::path::Path;

use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::policy;
use crate::policy::diagnostics::{
    CloneDiagnostic, CoverageDiagnostic, DamageDiagnostic, FindingDiagnostic, PolicyDiagnostic,
    SuppressionDiagnostic,
};

/// What the candidate side of a check is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckMode {
    /// Base is `HEAD`, candidate is the Git index.
    Staged,
    /// Base is `merge-base(HEAD, reference)`; candidate is `HEAD`, or the
    /// worktree overlay when `worktree` is set.
    Base { reference: String, worktree: bool },
}

/// One check invocation.
#[derive(Debug, Clone)]
pub struct CheckRequest<'a> {
    /// The repository work-tree directory, opened as is (no upward search).
    pub repository: &'a Path,
    pub mode: CheckMode,
    /// A trusted configuration file replacing repository policy.
    pub config_path: Option<&'a Path>,
    pub allow_new_suppressions: bool,
}

/// One diagnostic of a check, whichever stage raised it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckDiagnostic {
    /// E101, E102 or G102.
    Complexity(PolicyDiagnostic),
    /// V101.
    Finding(FindingDiagnostic),
    /// S101 or S102.
    Suppression(SuppressionDiagnostic),
    /// A101.
    Damage(DamageDiagnostic),
    /// A102.
    Coverage(CoverageDiagnostic),
    /// V102.
    Clone(CloneDiagnostic),
    /// C101 or C102 about the candidate's `nsd.yml`.
    Config(policy::Diagnostic),
    /// A failure with no policy verdict behind it: G101 for a repository,
    /// snapshot, diff or read failure, or the code of a `resolve` failure.
    Failure { code: &'static str, message: String },
}

impl CheckDiagnostic {
    /// The stable `NSD-` code.
    pub fn code(&self) -> &'static str {
        match self {
            CheckDiagnostic::Complexity(found) => found.code,
            CheckDiagnostic::Finding(found) => found.code,
            CheckDiagnostic::Suppression(found) => found.code,
            CheckDiagnostic::Damage(found) => found.code,
            CheckDiagnostic::Coverage(found) => found.code,
            CheckDiagnostic::Clone(found) => found.code,
            CheckDiagnostic::Config(found) => found.code,
            CheckDiagnostic::Failure { code, .. } => code,
        }
    }
}

/// The complete diagnostic set of one check and its exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    /// Grouped in a fixed order: configuration, read failures, complexity,
    /// findings, suppressions, damage, coverage, clones; each group in its
    /// evaluator's own sorted order.
    pub diagnostics: Vec<CheckDiagnostic>,
    /// `policy::exit::exit_status` over `diagnostics`.
    pub exit_status: u8,
}

/// Runs one check. Never fails: an internal error is an `NSD-G101` (or the
/// `resolve` error's own code) in the outcome.
pub fn run_check(request: &CheckRequest<'_>) -> CheckOutcome {
    let _ = request;
    unimplemented!("M5-4 scaffold")
}

/// The bytes of an included entry that must be readable, or the G101 that
/// says its snapshot is unavailable.
fn required_bytes(
    path: &RepoPath,
    read: Result<Option<Vec<u8>>, GitError>,
) -> Result<Vec<u8>, CheckDiagnostic> {
    let _ = (path, read);
    unimplemented!("M5-4 scaffold")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::CODE_SNAPSHOT_UNAVAILABLE;

    #[test]
    fn test_unreadable_under_ceiling_changed_blob_maps_to_g101() {
        let path = RepoPath::from_bytes(b"src/Foo.java".to_vec());

        let unreadable = required_bytes(&path, Ok(None)).expect_err("an unreadable blob fails");
        let readable = required_bytes(&path, Ok(Some(b"class Foo {}".to_vec())));

        assert!(matches!(unreadable, CheckDiagnostic::Failure { .. }));
        assert_eq!(unreadable.code(), CODE_SNAPSHOT_UNAVAILABLE);
        assert_eq!(
            readable.expect("readable bytes pass through"),
            b"class Foo {}"
        );
    }
}

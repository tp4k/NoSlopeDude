//! M4-2: V102, the clone-regression policy (`nsd-plan-final.md`
//! *Diagnostics*).

use crate::config::PolicyConfig;
use crate::git::diff::Change;
use crate::git::GitError;
use crate::policy::diagnostics::CloneDiagnostic;
use crate::policy::findings::FindingFile;

/// Raises V102 for every changed candidate clone occurrence that is new or
/// materially extended and has a qualifying match elsewhere.
pub fn evaluate_clones(
    _base: &[FindingFile<'_>],
    _candidate: &[FindingFile<'_>],
    _unchanged: &[FindingFile<'_>],
    _changes: &[Change],
    _min_clone_lines: u32,
    _policy: &PolicyConfig,
) -> Result<Vec<CloneDiagnostic>, GitError> {
    todo!()
}

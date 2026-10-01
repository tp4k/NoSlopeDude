//! M3-4: A101, the parse-damage policy (`nsd-plan-final.md` *Diagnostics*).

use crate::git::diff::Change;
use crate::git::GitError;
use crate::policy::diagnostics::DamageDiagnostic;
use crate::policy::findings::FindingFile;

/// Raises A101 for every changed candidate file whose parse damage does not
/// map through unchanged source, and for every excluded callable that
/// contains an added line. Output is sorted by (path bytes, start line, end
/// line), exact duplicates collapsed.
pub fn evaluate_damage(
    base: &[FindingFile<'_>],
    candidate: &[FindingFile<'_>],
    changes: &[Change],
) -> Result<Vec<DamageDiagnostic>, GitError> {
    let _ = (base, candidate, changes);
    unimplemented!()
}

//! M3-4: A101, the parse-damage policy (`nsd-plan-final.md` *Diagnostics*).
//!
//! Damage is tolerated line by line only when it is the `LineMap` image of
//! base damage lines. Separately, a callable salvage excluded that contains an
//! added line is an unmeasured changed entity.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::git::diff::{map_lines, Change, LineMap};
use crate::git::path::RepoPath;
use crate::git::GitError;
use crate::ir::Span;
use crate::policy::diagnostics::{DamageDiagnostic, CODE_PARSE_DAMAGE};
use crate::policy::findings::FindingFile;

fn line_range(span: Span) -> (usize, usize) {
    (span.start_line as usize, span.end_line as usize)
}

fn diagnostic(path: &RepoPath, (start, end): (usize, usize)) -> DamageDiagnostic {
    DamageDiagnostic {
        code: CODE_PARSE_DAMAGE,
        candidate_path: path.clone(),
        candidate_start_line: start,
        candidate_end_line: end,
    }
}

/// The candidate lines that base damage lines map to; empty without a base.
fn tolerated_lines(base: Option<&FindingFile<'_>>, line_map: Option<&LineMap>) -> BTreeSet<usize> {
    let (Some(base), Some(line_map)) = (base, line_map) else {
        return BTreeSet::new();
    };
    base.analysis
        .ir
        .damage
        .iter()
        .flat_map(|damage| {
            let (start, end) = line_range(damage.span);
            line_map.base_to_candidate.range(start..=end)
        })
        .map(|(_, &line)| line)
        .collect()
}

fn covers(lines: &BTreeSet<usize>, (start, end): (usize, usize)) -> bool {
    lines.range(start..=end).count() == end - start + 1
}

fn file_diagnostics(
    base: Option<&FindingFile<'_>>,
    candidate: &FindingFile<'_>,
) -> Result<Vec<DamageDiagnostic>, GitError> {
    let ir = &candidate.analysis.ir;
    if ir.damage.is_empty() && ir.excluded_callables.is_empty() {
        return Ok(Vec::new());
    }
    let line_map = base
        .map(|base| map_lines(base.source, candidate.source))
        .transpose()?;
    let tolerated = tolerated_lines(base, line_map.as_ref());

    let mut raised: Vec<(usize, usize)> = ir
        .damage
        .iter()
        .map(|damage| line_range(damage.span))
        .filter(|&range| !covers(&tolerated, range))
        .collect();
    let damage_raised = raised.clone();
    for &span in &ir.excluded_callables {
        let (start, end) = line_range(span);
        let has_added_line = match &line_map {
            Some(map) => map
                .added_candidate_lines
                .range(start..=end)
                .next()
                .is_some(),
            None => true,
        };
        let already_reported = damage_raised
            .iter()
            .any(|&(damage_start, damage_end)| damage_start <= end && start <= damage_end);
        if has_added_line && !already_reported {
            raised.push((start, end));
        }
    }
    Ok(raised
        .into_iter()
        .map(|range| diagnostic(&candidate.path, range))
        .collect())
}

/// Raises A101 for every changed candidate file whose parse damage does not
/// map through unchanged source, and for every excluded callable that
/// contains an added line. Output is sorted by (path bytes, start line, end
/// line), exact duplicates collapsed.
pub fn evaluate_damage(
    base: &[FindingFile<'_>],
    candidate: &[FindingFile<'_>],
    changes: &[Change],
) -> Result<Vec<DamageDiagnostic>, GitError> {
    let base_by_path: HashMap<&RepoPath, &FindingFile<'_>> =
        base.iter().map(|file| (&file.path, file)).collect();
    let candidate_by_path: HashMap<&RepoPath, &FindingFile<'_>> =
        candidate.iter().map(|file| (&file.path, file)).collect();

    let mut sorted: BTreeMap<(&RepoPath, usize, usize), DamageDiagnostic> = BTreeMap::new();
    for change in changes {
        let (base_path, candidate_path) = match change {
            Change::Added { path, .. } => (None, path),
            Change::Modified { path, .. } | Change::Typechange { path, .. } => (Some(path), path),
            Change::Renamed { from, to, .. } => (Some(from), to),
            Change::Deleted { .. } => continue,
        };
        let Some(&file) = candidate_by_path.get(candidate_path) else {
            continue;
        };
        let base_file = base_path.and_then(|path| base_by_path.get(path).copied());
        for found in file_diagnostics(base_file, file)? {
            sorted.insert(
                (
                    candidate_path,
                    found.candidate_start_line,
                    found.candidate_end_line,
                ),
                found,
            );
        }
    }
    Ok(sorted.into_values().collect())
}

//! M3-4: A101, the parse-damage policy (`nsd-plan-final.md` *Diagnostics*).
//!
//! Damage is tolerated line by line only when it is the `LineMap` image of
//! base damage lines. Separately, a callable or block salvage excluded that
//! holds an added or deleted line, or a callable that now swallows a
//! previously measured one's lines, is an unmeasured changed entity.

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

/// The candidate callable ranges, with their start and end lines indexed
/// for the one-endpoint lookups.
struct CandidateCallables {
    ranges: BTreeSet<(usize, usize)>,
    starts: BTreeSet<usize>,
    ends: BTreeSet<usize>,
}

impl CandidateCallables {
    fn new(candidate: &FindingFile<'_>) -> Self {
        let ranges: BTreeSet<(usize, usize)> = candidate
            .analysis
            .ir
            .callables
            .iter()
            .map(|callable| line_range(callable.span))
            .collect();
        let starts = ranges.iter().map(|&(start, _)| start).collect();
        let ends = ranges.iter().map(|&(_, end)| end).collect();
        Self {
            ranges,
            starts,
            ends,
        }
    }
}

/// Whether some candidate callable range agrees with the image of every
/// endpoint that maps; an unmapped pair is not still measured.
fn is_still_measured(
    (start, end): (Option<&usize>, Option<&usize>),
    candidates: &CandidateCallables,
) -> bool {
    match (start, end) {
        (Some(&start), Some(&end)) => candidates.ranges.contains(&(start, end)),
        (Some(start), None) => candidates.starts.contains(start),
        (None, Some(end)) => candidates.ends.contains(end),
        (None, None) => false,
    }
}

/// The candidate lines that the lines of base measured callables no longer
/// measured in the candidate map to.
fn measured_lines(
    base: Option<&FindingFile<'_>>,
    candidate: &FindingFile<'_>,
    line_map: Option<&LineMap>,
) -> BTreeSet<usize> {
    let (Some(base), Some(line_map)) = (base, line_map) else {
        return BTreeSet::new();
    };
    let still_measured = CandidateCallables::new(candidate);
    base.analysis
        .ir
        .callables
        .iter()
        .filter(|callable| {
            let (start, end) = line_range(callable.span);
            let image = (
                line_map.base_to_candidate.get(&start),
                line_map.base_to_candidate.get(&end),
            );
            !is_still_measured(image, &still_measured)
        })
        .flat_map(|callable| {
            let (start, end) = line_range(callable.span);
            line_map.base_to_candidate.range(start..=end)
        })
        .map(|(_, &line)| line)
        .collect()
}

/// Whether the candidate lines `start..=end` hold an added line, or sit on a
/// base stretch with a deleted line strictly between its first and last
/// mapped lines. Without a line map (an added file) every span qualifies.
fn has_changed_line(
    line_map: Option<&LineMap>,
    candidate_to_base: &BTreeMap<usize, usize>,
    (start, end): (usize, usize),
) -> bool {
    let Some(map) = line_map else {
        return true;
    };
    if map
        .added_candidate_lines
        .range(start..=end)
        .next()
        .is_some()
    {
        return true;
    }
    let mut mapped = candidate_to_base.range(start..=end);
    let (Some((_, &first)), Some((_, &last))) = (mapped.next(), mapped.next_back()) else {
        return false;
    };
    map.deleted_base_lines
        .range(first + 1..last)
        .next()
        .is_some()
}

fn covers(lines: &BTreeSet<usize>, (start, end): (usize, usize)) -> bool {
    lines.range(start..=end).count() == end - start + 1
}

fn file_diagnostics(
    base: Option<&FindingFile<'_>>,
    candidate: &FindingFile<'_>,
) -> Result<Vec<DamageDiagnostic>, GitError> {
    let ir = &candidate.analysis.ir;
    if ir.damage.is_empty() && ir.excluded_callables.is_empty() && ir.excluded_blocks.is_empty() {
        return Ok(Vec::new());
    }
    let line_map = base
        .map(|base| map_lines(base.source, candidate.source))
        .transpose()?;
    let tolerated = tolerated_lines(base, line_map.as_ref());
    let measured = measured_lines(base, candidate, line_map.as_ref());
    let candidate_to_base: BTreeMap<usize, usize> = line_map
        .iter()
        .flat_map(|map| map.base_to_candidate.iter())
        .map(|(&base_line, &candidate_line)| (candidate_line, base_line))
        .collect();

    let mut raised: Vec<(usize, usize)> = ir
        .damage
        .iter()
        .map(|damage| line_range(damage.span))
        .filter(|&range| !covers(&tolerated, range))
        .collect();
    let damage_raised = raised.clone();
    let callable_ranges: Vec<(usize, usize)> = ir
        .excluded_callables
        .iter()
        .map(|&span| line_range(span))
        .collect();
    for &(start, end) in &callable_ranges {
        let changed = has_changed_line(line_map.as_ref(), &candidate_to_base, (start, end));
        let swallows_measured = measured.range(start..=end).next().is_some();
        let already_reported = damage_raised
            .iter()
            .any(|&(damage_start, damage_end)| damage_start <= end && start <= damage_end);
        if (changed || swallows_measured) && !already_reported {
            raised.push((start, end));
        }
    }
    for &span in &ir.excluded_blocks {
        let (start, end) = line_range(span);
        let inside_callable = callable_ranges
            .iter()
            .any(|&(callable_start, callable_end)| callable_start <= start && end <= callable_end);
        let already_reported = raised
            .iter()
            .any(|&(raised_start, raised_end)| raised_start <= end && start <= raised_end);
        if !inside_callable
            && !already_reported
            && has_changed_line(line_map.as_ref(), &candidate_to_base, (start, end))
        {
            raised.push((start, end));
        }
    }
    Ok(raised
        .into_iter()
        .map(|range| diagnostic(&candidate.path, range))
        .collect())
}

/// Raises A101 for every changed candidate file whose parse damage does not
/// map through unchanged source, for every excluded callable or block with an
/// added or deleted line, and for every excluded callable that swallows a
/// previously measured one. Output is sorted by (path bytes, start line, end
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

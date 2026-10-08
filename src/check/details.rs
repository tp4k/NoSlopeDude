//! The sections of the canonical check document beyond the diagnostics
//! (M6-2, check scope): snapshot IDs, fingerprints, skip counts, changed
//! entities, family summaries and per-file coverage with parser gaps. Plain
//! data only; `format` turns it into JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::git::discovery::SkippedEntry;
use crate::git::path::RepoPath;
use crate::metrics;
use crate::model::{Callable, CloneGroup, FileLanguageLines, LanguageFamily, RuleFinding};
use crate::policy::damage::MappedGap;
use crate::rules;

/// Every key of the document's `skipped` object, sorted. Zero counts are
/// emitted, so the key set never depends on the repository.
pub const SKIP_KEYS: [&str; 13] = [
    "builtin_exclusion",
    "config_exclude",
    "invalid_encoding",
    "nested_checkout",
    "non_utf8_path",
    "outside_include",
    "parse_syntax_error",
    "parser_unavailable",
    "special_file",
    "submodule",
    "symlink",
    "too_large",
    "unsupported_extension",
];

/// The key of `skipped` counting files whose parse left damage.
pub const PARSE_SYNTAX_ERROR: &str = "parse_syntax_error";

/// A line range, 1-based and inclusive.
pub type LineSpan = (usize, usize);

/// The matched base callable behind an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityBase {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
}

/// One changed candidate callable: new, or matched to a base callable whose
/// identity or body differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub path: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
    pub base: Option<EntityBase>,
}

/// One family's scores.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilySummary {
    pub erosion: f64,
    pub flagged_lines: usize,
    pub scanned_lines: usize,
    pub unanalyzed_lines: usize,
    pub complete: bool,
    pub ratio: f64,
}

/// The settled scope's scores: `full` when unchanged files are analyzed
/// (V102 not off), `changed` otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Summaries {
    pub scope: &'static str,
    pub files: usize,
    pub overall: FamilySummary,
    pub java: FamilySummary,
    pub js_ts: FamilySummary,
}

/// One parse-damage span of a candidate file, mapped to its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub base: Option<LineSpan>,
    pub candidate: LineSpan,
    pub tolerated: bool,
}

impl From<MappedGap> for Gap {
    fn from(gap: MappedGap) -> Self {
        Gap {
            base: gap.base,
            candidate: gap.candidate,
            tolerated: gap.tolerated,
        }
    }
}

/// One changed file's analysis coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCoverage {
    pub path: String,
    pub analyzed_lines: usize,
    pub unanalyzed_lines: usize,
    pub complete: bool,
    pub gaps: Vec<Gap>,
}

/// Everything the canonical document holds besides its four base keys.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckDetails {
    pub base_snapshot: String,
    pub candidate_snapshot: String,
    pub measurement_fingerprint: String,
    pub configuration_fingerprint: String,
    pub skipped: BTreeMap<&'static str, usize>,
    pub entities: Vec<Entity>,
    pub summaries: Summaries,
    pub coverage: Vec<FileCoverage>,
}

/// What one analyzed candidate file contributes to the summaries and
/// coverage, whether it was analyzed this run or restored from the cache.
#[derive(Debug, Clone)]
pub(super) struct FileFacts {
    pub(super) path: RepoPath,
    pub(super) language: LanguageFamily,
    /// The distinct executable lines that survived lowering.
    pub(super) executable_lines: Vec<usize>,
    pub(super) unanalyzed_lines: usize,
    pub(super) damaged: bool,
    callables: Vec<Callable>,
    findings: Vec<RuleFinding>,
}

impl FileFacts {
    pub(super) fn new(
        path: RepoPath,
        language: LanguageFamily,
        callables: Vec<Callable>,
        mut findings: Vec<RuleFinding>,
        executable_lines: Vec<usize>,
        unanalyzed_lines: usize,
        damaged: bool,
    ) -> Self {
        // One key for the file's lines and its findings, whatever spelling
        // the analysis gave the path: the rendered repository path.
        let key = PathBuf::from(path.render());
        for finding in &mut findings {
            finding.relative_path = key.clone();
        }
        FileFacts {
            path,
            language,
            executable_lines,
            unanalyzed_lines,
            damaged,
            callables,
            findings,
        }
    }

    pub(super) fn scanned_lines(&self) -> usize {
        self.executable_lines.len()
    }
}

/// `skipped` with every key present: the discovery skips of the candidate
/// snapshot, the analysis reasons seen on the candidate side, and the files
/// whose parse left damage.
pub(super) fn skip_counts(
    discovery: &[SkippedEntry],
    reasons: &[&'static str],
    parse_syntax_errors: usize,
) -> BTreeMap<&'static str, usize> {
    let mut counts: BTreeMap<&'static str, usize> = SKIP_KEYS.iter().map(|key| (*key, 0)).collect();
    let labels = discovery
        .iter()
        .map(|skipped| skipped.reason.label())
        .chain(reasons.iter().copied());
    for label in labels {
        *counts.entry(label).or_insert(0) += 1;
    }
    counts.insert(PARSE_SYNTAX_ERROR, parse_syntax_errors);
    counts
}

/// A float as the canonical document spells it: finite, and never `-0.0`.
fn normalize_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// The summaries over `facts` (any order; sorted here by path so the erosion
/// sums add in one fixed order). `clone_groups` are the groups over exactly
/// this file set, so the numerator is scan's. `incomplete` names the
/// families that lost a file to an analysis failure.
pub(super) fn summarize(
    scope: &'static str,
    mut facts: Vec<&FileFacts>,
    clone_groups: &[CloneGroup],
    incomplete: &[LanguageFamily],
) -> Summaries {
    facts.sort_by(|a, b| a.path.cmp(&b.path));
    let lines: Vec<FileLanguageLines> = facts
        .iter()
        .map(|file| FileLanguageLines {
            relative_path: PathBuf::from(file.path.render()),
            language: file.language,
            scanned_lines: file.scanned_lines(),
            unanalyzed_lines: file.unanalyzed_lines,
            executable_lines: file.executable_lines.clone(),
        })
        .collect();
    let findings: Vec<RuleFinding> = facts
        .iter()
        .flat_map(|file| file.findings.iter().cloned())
        .collect();
    let verbosity = rules::compute_verbosity(&lines, &findings, clone_groups);
    let family = |language: Option<LanguageFamily>| {
        let in_family = |file: &&&FileFacts| language.is_none_or(|wanted| file.language == wanted);
        let callables: Vec<Callable> = facts
            .iter()
            .filter(in_family)
            .flat_map(|file| file.callables.iter().cloned())
            .collect();
        let damaged = facts.iter().filter(in_family).any(|file| file.damaged);
        let lost = incomplete
            .iter()
            .any(|lost| language.is_none_or(|wanted| *lost == wanted));
        (
            normalize_zero(metrics::erosion(&callables)),
            damaged || lost,
        )
    };
    let summary = |language, score: crate::model::VerbosityScore| {
        let (erosion, broken) = family(language);
        FamilySummary {
            erosion,
            flagged_lines: score.flagged_lines,
            scanned_lines: score.scanned_lines,
            unanalyzed_lines: score.unanalyzed_lines,
            complete: score.unanalyzed_lines == 0 && !broken,
            ratio: normalize_zero(score.ratio),
        }
    };
    Summaries {
        scope,
        files: facts.len(),
        overall: summary(None, verbosity.overall),
        java: summary(Some(LanguageFamily::Java), verbosity.java),
        js_ts: summary(Some(LanguageFamily::JsTs), verbosity.js_ts),
    }
}

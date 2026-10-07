//! Stage 5 (WS-5): one aggregation of every earlier stage's results into a
//! `Report`, then three renderings that all read from it so they cannot
//! drift — `report.json` (`render_json`), the standalone `report.html`
//! (`html::render_html`) and the terminal summary `main.rs` prints
//! (`terminal_summary`). This module never recomputes a score another
//! stage already published; the one exception is per-family erosion
//! (`build_scores`), which calls WS-2's own public `metrics::erosion`
//! again on a language-filtered slice of its callables — the same formula,
//! not a second implementation of it, since `MetricsResult.erosion` itself
//! is overall-only.

mod html;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::discover::DiscoverResult;
use crate::metrics;
use crate::model::{
    Callable, CloneGroup, ClonesResult, FileScanSummary, LanguageFamily, MetricsResult,
    ParseFailure, ParseFailureReason, RuleFinding, RuleId, RulesResult, ScanSettings, Target,
    VerbosityScore,
};

pub use html::render_html;

/// The GitHub revision info a scan carries (D6), independent of `model`'s
/// `Revision` only so the JSON shape is this module's own to evolve.
#[derive(Debug, Clone, Serialize)]
pub struct ReportRevision {
    pub sha: Option<String>,
    pub dirty: Option<bool>,
    pub unavailable_reason: Option<String>,
}

/// D17's recorded scan settings: the flags actually used, plus the raw
/// target string and its resolved revision (D6).
#[derive(Debug, Clone, Serialize)]
pub struct ReportScan {
    pub target: String,
    pub revision: ReportRevision,
    pub include_tests: bool,
    pub exclude: Vec<String>,
    pub min_clone_lines: u32,
}

/// D23's verbosity fraction, carried into the report verbatim.
#[derive(Debug, Clone, Serialize)]
pub struct ReportVerbosity {
    pub flagged_lines: usize,
    /// The analyzed executable lines, the ratio's denominator.
    pub scanned_lines: usize,
    /// The executable lines lowering pruned, which stay out of the
    /// denominator.
    pub unanalyzed_lines: usize,
    /// Whether every file of this family was analyzed in full.
    pub complete: bool,
    pub ratio: f64,
}

impl ReportVerbosity {
    /// `score` plus completeness: every executable line analyzed
    /// (`unanalyzed_lines == 0`) and no file of the scope lost to a parse
    /// failure (`parse_failed`). A total parse loss has no line count, so
    /// the count alone cannot tell.
    fn new(score: &VerbosityScore, parse_failed: bool) -> Self {
        ReportVerbosity {
            flagged_lines: score.flagged_lines,
            scanned_lines: score.scanned_lines,
            unanalyzed_lines: score.unanalyzed_lines,
            complete: score.unanalyzed_lines == 0 && !parse_failed,
            ratio: score.ratio,
        }
    }
}

/// D13/D23 for one language family (or overall): erosion plus verbosity.
#[derive(Debug, Clone, Serialize)]
pub struct ReportFamilyScores {
    pub erosion: f64,
    pub verbosity: ReportVerbosity,
}

/// D20: every score, overall and per language family.
#[derive(Debug, Clone, Serialize)]
pub struct ReportScores {
    pub overall: ReportFamilyScores,
    pub java: ReportFamilyScores,
    pub js_ts: ReportFamilyScores,
}

/// One reported source span: where it is, its D19-escaped-on-render
/// source excerpt (read back off disk by line span, D6/never re-parsed),
/// and a link to it — a full GitHub blob URL at the scanned revision for a
/// remote target, or a repo-relative `path#Lstart-Lend` label for a local
/// one (D6).
#[derive(Debug, Clone, Serialize)]
pub struct SourceLocation {
    pub relative_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub excerpt: String,
    pub link: String,
    pub is_remote_link: bool,
}

/// One D22 rule finding, rendered.
#[derive(Debug, Clone, Serialize)]
pub struct ReportFinding {
    pub rule_id: RuleId,
    pub language: &'static str,
    pub location: SourceLocation,
    pub flagged_lines: Vec<usize>,
}

/// One D14 clone group, rendered: every occurrence, in the same canonical
/// order `redundant_occurrences` relies on, so `locations[0]` is still the
/// group's first (non-redundant) occurrence.
#[derive(Debug, Clone, Serialize)]
pub struct ReportDuplicateGroup {
    pub language: &'static str,
    pub redundant_lines: usize,
    pub locations: Vec<SourceLocation>,
}

/// One top-25 (by `cc`) callable, rendered. `location` spans
/// `[start_line, end_line]` of the whole callable node
/// (`Callable::start_line`/`end_line`, from `IrCallable::span`, not
/// `body_span`) — its real last line, through the closing line of its own
/// body, not just its declaration's first line (M0c-13).
#[derive(Debug, Clone, Serialize)]
pub struct ReportCallable {
    pub name: String,
    pub language: &'static str,
    pub cc: u32,
    pub sloc: usize,
    pub mass: f64,
    pub location: SourceLocation,
}

/// A discovery-time skip (D16) or a parse failure (D18), told apart by
/// `reason`'s `parse_` prefix on the latter. Every row contributes nothing
/// to the scores except a salvaged `parse_syntax_error` one, whose `detail`
/// starts with `salvaged`.
#[derive(Debug, Clone, Serialize)]
pub struct ReportSkippedFile {
    pub relative_path: PathBuf,
    pub reason: String,
    pub detail: Option<String>,
    /// A salvaged file's mapped parser gaps, sorted by start line; absent on
    /// every other row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gaps: Option<Vec<ReportGap>>,
    /// A salvaged file's count of excluded (unmeasured) callables; absent on
    /// every other row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmeasured_callables: Option<usize>,
}

/// One mapped parser gap: an inclusive 1-based line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReportGap {
    pub start_line: usize,
    pub end_line: usize,
}

/// The Assumptions-mandated adaptation label: this scanner's CC/verbosity
/// rules are its own documented adaptation, not a claim of numerical
/// equivalence to scb-check's Python scores.
#[derive(Debug, Clone, Serialize)]
pub struct ReportAdaptation {
    pub summary: String,
    pub cc_rules_doc: String,
    pub wasteful_rules_doc: String,
}

const ADAPTATION_SUMMARY: &str = "CC, SLOC, mass, erosion and verbosity are computed from this scanner's own Java/JS-TS AST rules, adapted from scb-check's Python-only formulas. No numerical equivalence to scb-check's own scores is claimed.";
const CC_RULES_DOC: &str = "docs/cc-rules.md";
const WASTEFUL_RULES_DOC: &str = "docs/wasteful-rules.md";

impl Default for ReportAdaptation {
    fn default() -> Self {
        ReportAdaptation {
            summary: ADAPTATION_SUMMARY.to_string(),
            cc_rules_doc: CC_RULES_DOC.to_string(),
            wasteful_rules_doc: WASTEFUL_RULES_DOC.to_string(),
        }
    }
}

/// Everything one scan produces, in the shape every rendering shares.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub scan: ReportScan,
    pub scores: ReportScores,
    pub findings: Vec<ReportFinding>,
    pub duplicates: Vec<ReportDuplicateGroup>,
    pub top25: Vec<ReportCallable>,
    pub skipped_files: Vec<ReportSkippedFile>,
    pub incomplete: bool,
    pub adaptation: ReportAdaptation,
}

/// Everything `aggregate` needs, borrowed from the pipeline's stage
/// outputs — never a second computation of any of it.
pub struct ReportInput<'a> {
    pub target: &'a Target,
    pub target_input: &'a str,
    pub root: &'a Path,
    pub revision: &'a crate::model::Revision,
    pub settings: &'a ScanSettings,
    pub discover: &'a DiscoverResult,
    pub parse_failures: &'a [ParseFailure],
    pub metrics: &'a MetricsResult,
    pub clones: &'a ClonesResult,
    pub rules: &'a RulesResult,
}

/// The one aggregation every rendering reads from.
pub fn aggregate(input: &ReportInput) -> Report {
    // C7: one file's lines are read and split at most once per `aggregate`
    // call, no matter how many findings/duplicate locations/top25 callables
    // land in it -- see `read_excerpt`'s own doc comment. Lives only for
    // this call (a stack-local `HashMap`, threaded down by `&mut`
    // reference), not a `ReportInput` field or anything cached across runs.
    let mut excerpt_cache: ExcerptCache = HashMap::new();
    Report {
        scan: build_scan(input),
        scores: build_scores(input),
        findings: input
            .rules
            .findings
            .iter()
            .map(|f| build_finding(input, f, &mut excerpt_cache))
            .collect(),
        duplicates: input
            .clones
            .groups
            .iter()
            .map(|group| build_duplicate_group(input, group, &mut excerpt_cache))
            .collect(),
        top25: input
            .metrics
            .top25
            .iter()
            .map(|c| build_callable(input, c, &mut excerpt_cache))
            .collect(),
        skipped_files: build_skipped_files(input),
        // WS-6's `SkipReason` split: a discovery-time skip only marks the
        // scan incomplete when it is an analysis failure (`Unreadable` --
        // the walk tried this path and could not read it), never a policy
        // exclusion (D16, a user `--exclude`) the scan behaved exactly as
        // configured against.
        incomplete: input.metrics.incomplete
            || input.rules.incomplete
            || input
                .discover
                .skipped
                .iter()
                .any(|file| file.reason.is_analysis_failure()),
        adaptation: ReportAdaptation::default(),
    }
}

/// Runs the report stage: aggregates, writes `report.json` and
/// `report.html` into `settings.output` (D17), and returns the aggregate
/// for `main.rs`'s terminal rendering. An I/O failure here is one of D18's
/// three reserved fatal conditions (an unwritable output directory).
pub fn run(input: &ReportInput) -> anyhow::Result<Report> {
    let report = aggregate(input);
    let json = render_json(&report)?;
    let json_path = input.settings.output.join("report.json");
    fs::write(&json_path, json).with_context(|| format!("cannot write {}", json_path.display()))?;

    let html = render_html(&report);
    let html_path = input.settings.output.join("report.html");
    fs::write(&html_path, html).with_context(|| format!("cannot write {}", html_path.display()))?;

    Ok(report)
}

/// D17: `report.json` as pretty-printed JSON.
pub fn render_json(report: &Report) -> anyhow::Result<String> {
    serde_json::to_string_pretty(report).context("failed to serialize the report to JSON")
}

fn build_scan(input: &ReportInput) -> ReportScan {
    ReportScan {
        target: input.target_input.to_string(),
        revision: ReportRevision {
            sha: input.revision.sha.clone(),
            dirty: input.revision.dirty,
            unavailable_reason: input.revision.unavailable_reason.clone(),
        },
        include_tests: input.settings.include_tests,
        exclude: input.settings.exclude.clone(),
        min_clone_lines: input.settings.min_clone_lines,
    }
}

fn build_scores(input: &ReportInput) -> ReportScores {
    let java_callables: Vec<Callable> = input
        .metrics
        .callables
        .iter()
        .filter(|callable| callable.language == LanguageFamily::Java)
        .cloned()
        .collect();
    let js_ts_callables: Vec<Callable> = input
        .metrics
        .callables
        .iter()
        .filter(|callable| callable.language == LanguageFamily::JsTs)
        .cloned()
        .collect();

    let parse_failed = |family: LanguageFamily| {
        input.parse_failures.iter().any(|failure| {
            failure
                .relative_path
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(LanguageFamily::from_extension)
                == Some(family)
        })
    };

    ReportScores {
        overall: ReportFamilyScores {
            erosion: normalize_zero(input.metrics.erosion),
            verbosity: ReportVerbosity::new(
                &input.rules.verbosity.overall,
                !input.parse_failures.is_empty(),
            ),
        },
        java: ReportFamilyScores {
            erosion: normalize_zero(metrics::erosion(&java_callables)),
            verbosity: ReportVerbosity::new(
                &input.rules.verbosity.java,
                parse_failed(LanguageFamily::Java),
            ),
        },
        js_ts: ReportFamilyScores {
            erosion: normalize_zero(metrics::erosion(&js_ts_callables)),
            verbosity: ReportVerbosity::new(
                &input.rules.verbosity.js_ts,
                parse_failed(LanguageFamily::JsTs),
            ),
        },
    }
}

/// `metrics::erosion` sums an empty filtered slice when no callable exceeds
/// the CC threshold; on this toolchain `Iterator::sum` over an empty `f64`
/// sequence is `-0.0`, and `-0.0 / total_mass` stays `-0.0`. `-0.0 == 0.0`
/// numerically, so this changes no score's value — it only normalizes the
/// sign of an exact zero so a rendering never reads "-0.0000".
fn normalize_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

fn build_finding(
    input: &ReportInput,
    finding: &RuleFinding,
    excerpt_cache: &mut ExcerptCache,
) -> ReportFinding {
    ReportFinding {
        rule_id: finding.rule_id,
        language: family_label(finding.language),
        location: build_location(
            input,
            &finding.relative_path,
            finding.start_line,
            finding.end_line,
            excerpt_cache,
        ),
        flagged_lines: finding.flagged_lines.clone(),
    }
}

fn build_duplicate_group(
    input: &ReportInput,
    group: &CloneGroup,
    excerpt_cache: &mut ExcerptCache,
) -> ReportDuplicateGroup {
    ReportDuplicateGroup {
        language: family_label(group.language),
        redundant_lines: group.redundant_lines,
        locations: group
            .locations
            .iter()
            .map(|location| {
                build_location(
                    input,
                    &location.relative_path,
                    location.start_line,
                    location.end_line,
                    excerpt_cache,
                )
            })
            .collect(),
    }
}

fn build_callable(
    input: &ReportInput,
    callable: &Callable,
    excerpt_cache: &mut ExcerptCache,
) -> ReportCallable {
    ReportCallable {
        name: callable.name.clone(),
        language: family_label(callable.language),
        cc: callable.cc,
        sloc: callable.sloc,
        mass: callable.mass,
        location: build_location(
            input,
            &callable.relative_path,
            callable.start_line,
            callable.end_line,
            excerpt_cache,
        ),
    }
}

fn build_skipped_files(input: &ReportInput) -> Vec<ReportSkippedFile> {
    let mut skipped: Vec<ReportSkippedFile> = input
        .discover
        .skipped
        .iter()
        .map(|file| ReportSkippedFile {
            relative_path: file.relative_path.clone(),
            reason: file.reason.label().to_string(),
            detail: None,
            gaps: None,
            unmeasured_callables: None,
        })
        .collect();
    let summaries: HashMap<&Path, &FileScanSummary> = input
        .metrics
        .file_scan_summaries
        .iter()
        .map(|summary| (summary.relative_path.as_path(), summary))
        .collect();
    skipped.extend(input.parse_failures.iter().map(|failure| {
        // Only a salvaged file has a summary to name its gaps from.
        let summary = summaries.get(failure.relative_path.as_path());
        ReportSkippedFile {
            relative_path: failure.relative_path.clone(),
            reason: format!("parse_{}", failure.reason.label()),
            detail: skipped_detail(failure),
            gaps: summary.map(|summary| {
                summary
                    .gaps
                    .iter()
                    .map(|gap| ReportGap {
                        start_line: gap.start_line,
                        end_line: gap.end_line,
                    })
                    .collect()
            }),
            unmeasured_callables: summary.map(|summary| summary.unmeasured_callables),
        }
    }));
    skipped.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    skipped
}

/// WS-6 salvage: a `SyntaxError` file still has a `ParsedFile` and is
/// scored except for its damaged entities, so its row says `salvaged` --
/// it is listed here only so the file that made the report `incomplete` is
/// named. Every other parse-failure reason means no `ParsedFile` at all.
fn skipped_detail(failure: &ParseFailure) -> Option<String> {
    if failure.reason != ParseFailureReason::SyntaxError {
        return failure.detail.clone();
    }
    Some(match &failure.detail {
        Some(detail) => format!("salvaged; {detail}"),
        None => "salvaged".to_string(),
    })
}

fn build_location(
    input: &ReportInput,
    relative_path: &Path,
    start_line: usize,
    end_line: usize,
    excerpt_cache: &mut ExcerptCache,
) -> SourceLocation {
    let excerpt = read_excerpt(
        excerpt_cache,
        input.root,
        relative_path,
        start_line,
        end_line,
    );
    let (link, is_remote_link) = match source_link(
        input.target,
        input.revision,
        relative_path,
        start_line,
        end_line,
    ) {
        Some(url) => (url, true),
        None => (local_link_label(relative_path, start_line, end_line), false),
    };
    SourceLocation {
        relative_path: relative_path.to_path_buf(),
        start_line,
        end_line,
        excerpt,
        link,
        is_remote_link,
    }
}

/// C7: `read_excerpt`'s per-file read/split, keyed by `relative_path`, live
/// only for one `aggregate` call (a stack-local in `aggregate` itself,
/// threaded down by `&mut` reference -- never a `ReportInput` field or
/// anything `pub`). An unreadable file caches as an empty `Vec`, which
/// `read_excerpt` below treats the same way a fresh failed read always
/// did (`end_index` collapses to 0, so `start_index >= end_index` and it
/// returns an empty excerpt), so a repeatedly-referenced unreadable path
/// is not repeatedly retried either.
type ExcerptCache = HashMap<PathBuf, Vec<String>>;

/// This call's own already-read lines for `relative_path`, reading and
/// splitting it off disk only on the first request for that path.
fn cached_lines<'a>(
    excerpt_cache: &'a mut ExcerptCache,
    root: &Path,
    relative_path: &Path,
) -> &'a [String] {
    excerpt_cache
        .entry(relative_path.to_path_buf())
        .or_insert_with(|| {
            let Ok(text) = fs::read_to_string(root.join(relative_path)) else {
                return Vec::new();
            };
            text.lines().map(str::to_string).collect()
        })
}

/// Reads `relative_path`'s `[start_line, end_line]` span off disk (never
/// re-parsed, D6/anti-scope), by way of `excerpt_cache` above. An I/O
/// failure reading a file that already parsed successfully is not one of
/// D18's three reserved fatal cases, so it degrades to an empty excerpt
/// rather than failing the whole report.
fn read_excerpt(
    excerpt_cache: &mut ExcerptCache,
    root: &Path,
    relative_path: &Path,
    start_line: usize,
    end_line: usize,
) -> String {
    let lines = cached_lines(excerpt_cache, root, relative_path);
    let start_index = start_line.saturating_sub(1);
    let end_index = end_line.min(lines.len());
    if start_index >= end_index {
        return String::new();
    }
    lines[start_index..end_index].join("\n")
}

fn local_link_label(relative_path: &Path, start_line: usize, end_line: usize) -> String {
    format!("{}#L{start_line}-L{end_line}", relative_path.display())
}

const GITHUB_HTTPS_PREFIX: &str = "https://github.com/";
const GITHUB_HTTP_PREFIX: &str = "http://github.com/";

/// D6: a full GitHub blob link at the scanned revision, for a remote
/// target whose sha is known; `None` otherwise (a local target, or a
/// remote one whose sha could not be resolved).
fn source_link(
    target: &Target,
    revision: &crate::model::Revision,
    relative_path: &Path,
    start_line: usize,
    end_line: usize,
) -> Option<String> {
    let Target::Remote(remote) = target else {
        return None;
    };
    let sha = revision.sha.as_deref()?;
    let (owner, repo) = parse_github_owner_repo(&remote.url)?;
    Some(format!(
        "https://github.com/{owner}/{repo}/blob/{sha}/{path}#L{start_line}-L{end_line}",
        path = percent_encode_path(relative_path)
    ))
}

/// RFC 3986 percent-encoding for a GitHub blob URL's path: every segment's
/// bytes outside `A-Za-z0-9._~-` become `%XX` (uppercase hex); `/` is kept
/// as the path's own separator, never encoded itself.
fn percent_encode_path(relative_path: &Path) -> String {
    relative_path
        .to_string_lossy()
        .split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'-' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix(GITHUB_HTTPS_PREFIX)
        .or_else(|| url.strip_prefix(GITHUB_HTTP_PREFIX))?;
    let rest = rest.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = rest.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn family_label(language: LanguageFamily) -> &'static str {
    match language {
        LanguageFamily::Java => "java",
        LanguageFamily::JsTs => "js_ts",
    }
}

/// The terminal rendering `main.rs` prints: the CLI-level acceptance test's
/// own words are "carrying the overall and per-family erosion and
/// verbosity scores", so those are the numbers this always shows, plus the
/// scan settings and every count bullet 5 names.
pub fn terminal_summary(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let _ = writeln!(out, "Scan settings:");
    let _ = writeln!(out, "  target: {}", report.scan.target);
    let _ = write!(
        out,
        "  revision: {}",
        report.scan.revision.sha.as_deref().unwrap_or(
            report
                .scan
                .revision
                .unavailable_reason
                .as_deref()
                .unwrap_or("unknown")
        )
    );
    // B9: mirrors the ` (dirty: {dirty})` suffix html.rs:70-72 already
    // appends to the same line in report.html; `report.json` has always
    // carried the flag as its own field. `None` (no git repository) prints
    // nothing extra, same as there.
    if let Some(dirty) = report.scan.revision.dirty {
        let _ = write!(out, " (dirty: {dirty})");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  include_tests: {}", report.scan.include_tests);
    let _ = writeln!(out, "  exclude: {:?}", report.scan.exclude);
    let _ = writeln!(out, "  min_clone_lines: {}", report.scan.min_clone_lines);
    let _ = writeln!(out);

    let _ = writeln!(out, "Scores (erosion / verbosity ratio):");
    let _ = writeln!(
        out,
        "  overall:  {:.4} / {:.4}",
        report.scores.overall.erosion, report.scores.overall.verbosity.ratio
    );
    let _ = writeln!(
        out,
        "  java:     {:.4} / {:.4}",
        report.scores.java.erosion, report.scores.java.verbosity.ratio
    );
    let _ = writeln!(
        out,
        "  js_ts:    {:.4} / {:.4}",
        report.scores.js_ts.erosion, report.scores.js_ts.verbosity.ratio
    );
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "Findings: {}  Duplicate groups: {}  Top callables: {}  Skipped files: {}",
        report.findings.len(),
        report.duplicates.len(),
        report.top25.len(),
        report.skipped_files.len()
    );
    let _ = writeln!(
        out,
        "Incomplete (a file failed to parse): {}",
        report.incomplete
    );
    let _ = writeln!(
        out,
        "Adaptation: {} See {} and {}.",
        report.adaptation.summary,
        report.adaptation.cc_rules_doc,
        report.adaptation.wasteful_rules_doc
    );

    out
}

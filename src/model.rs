//! Shared types used across every scan stage.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Default value for `--min-clone-lines`: how many duplicated source lines
/// make a clone worth reporting. The user can change this at run time.
pub const DEFAULT_MIN_CLONE_LINES: u32 = 10;

/// Fixed cyclomatic-complexity threshold from the published erosion formula:
/// a callable with `CC` above this contributes to erosion mass. This is a
/// different concept from `DEFAULT_MIN_CLONE_LINES` even though both are 10
/// today, and it is not user-configurable.
pub const CC_EROSION_THRESHOLD: u32 = 10;

/// Parsed and validated settings for one scan run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSettings {
    pub output: PathBuf,
    pub include_tests: bool,
    pub exclude: Vec<String>,
    pub min_clone_lines: u32,
}

/// A scan target as given on the command line, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Local(PathBuf),
    Remote(RemoteTarget),
}

/// A public GitHub repository URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTarget {
    pub url: String,
}

/// The revision of the scanned root, recorded on a best-effort basis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Revision {
    pub sha: Option<String>,
    pub dirty: Option<bool>,
    pub unavailable_reason: Option<String>,
}

/// The two language families the scanner groups the seven scanned
/// extensions into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageFamily {
    Java,
    JsTs,
}

impl LanguageFamily {
    /// Maps a file extension (without the leading dot) to its language
    /// family, or `None` if the extension is not one of the seven scanned
    /// extensions.
    pub fn from_extension(extension: &str) -> Option<LanguageFamily> {
        match extension {
            "java" => Some(LanguageFamily::Java),
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Some(LanguageFamily::JsTs),
            _ => None,
        }
    }
}

/// A source file selected for scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
}

/// The D16 rule (or gitignore, a user `--exclude`, or an unreadable path)
/// that skipped a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    Gitignore,
    DependencyOrBuildOutput,
    GeneratedCode,
    Test,
    UserExclude,
    /// A per-entry walk error (D18: not one of the three reserved fatal
    /// cases), e.g. a subdirectory with no read permission.
    Unreadable,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::Gitignore => "gitignore",
            SkipReason::DependencyOrBuildOutput => "dependency_or_build_output",
            SkipReason::GeneratedCode => "generated_code",
            SkipReason::Test => "test",
            SkipReason::UserExclude => "user_exclude",
            SkipReason::Unreadable => "unreadable",
        }
    }
}

/// A file that was walked but not selected for scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub relative_path: PathBuf,
    pub reason: SkipReason,
}

/// The grammar within a `LanguageFamily` a file's extension selects (D20):
/// `js_ts` covers three distinct grammars, `java` covers one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    Java,
    JavaScript,
    TypeScript,
    Tsx,
}

impl Grammar {
    /// Maps a file extension (without the leading dot) to the grammar that
    /// parses it, or `None` if the extension is not one of the seven
    /// scanned extensions.
    pub fn for_extension(extension: &str) -> Option<Grammar> {
        match extension {
            "java" => Some(Grammar::Java),
            "ts" => Some(Grammar::TypeScript),
            "tsx" => Some(Grammar::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Grammar::JavaScript),
            _ => None,
        }
    }
}

/// Why a file failed to parse (D18), mirroring `SkipReason`'s typed shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseFailureReason {
    Unreadable,
    UnsupportedExtension,
    GrammarSetup,
    SyntaxError,
}

impl ParseFailureReason {
    pub fn label(self) -> &'static str {
        match self {
            ParseFailureReason::Unreadable => "unreadable",
            ParseFailureReason::UnsupportedExtension => "unsupported_extension",
            ParseFailureReason::GrammarSetup => "grammar_setup",
            ParseFailureReason::SyntaxError => "syntax_error",
        }
    }
}

/// A file that failed to parse (D18): a syntax error or an I/O failure.
/// Recorded separately from `SkippedFile` since it is a different
/// condition from a discovery-time skip — the scan continues past it and
/// the affected scores carry `incomplete: true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub relative_path: PathBuf,
    pub reason: ParseFailureReason,
    pub detail: Option<String>,
}

/// A callable extracted from a parsed file (D8): a method, constructor,
/// compact constructor, static initializer, lambda, arrow function,
/// function expression/declaration or class method that has a body.
#[derive(Debug, Clone, PartialEq)]
pub struct Callable {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    /// D10: the node's own name, else the enclosing declarator/pair/
    /// assignment's name, else `<anonymous>@<line>`.
    pub name: String,
    /// 1-based line of the callable's own declaration (not its body).
    pub start_line: usize,
    /// D7: `1 + decision points` in the callable's own body, excluding any
    /// nested callable's span (D9).
    pub cc: u32,
    /// D11: distinct executable source lines in the callable's own body.
    pub sloc: usize,
    /// D13: `cc as f64 * (sloc as f64).sqrt()`.
    pub mass: f64,
}

/// A block-shaped syntax node kept for WS-3's duplicate-block detection: a
/// `{ … }` scope found inside a callable's body (excluding nested
/// callables, D9), given as the whole file's span so a block can be
/// compared across files.
#[derive(Debug, Clone, PartialEq)]
pub struct SyntaxBlock {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub kind: &'static str,
    pub start_line: usize,
    pub end_line: usize,
}

/// D12: the number of distinct executable source lines (D11's per-line
/// rule) across one *whole* successfully parsed file — a different
/// quantity from Σ callable SLOC, since it also covers code outside every
/// callable (imports, field and type declarations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileScanSummary {
    pub relative_path: PathBuf,
    pub scanned_lines: usize,
}

/// Everything the metrics stage (WS-2) computed from every successfully
/// parsed file.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricsResult {
    pub callables: Vec<Callable>,
    pub syntax_blocks: Vec<SyntaxBlock>,
    pub file_scan_summaries: Vec<FileScanSummary>,
    /// D13: fraction of total mass held by callables with `cc >
    /// CC_EROSION_THRESHOLD`; `0.0` when the total mass is `0.0`.
    pub erosion: f64,
    /// Top 25 callables by `cc` descending, ties broken by path then start
    /// line.
    pub top25: Vec<Callable>,
    /// D18: whether at least one file failed to parse — the scores carry
    /// this marker rather than failing the scan.
    pub incomplete: bool,
}

/// One occurrence of a clone group (D14): where it is, and how many D11-
/// counted source lines it spans. `source_lines` is what WS-3's ranking
/// metric and WS-4's verbosity numerator (D23) both sum over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneLocation {
    pub relative_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub source_lines: usize,
}

/// A group of matching normalized code blocks (D14, D15): every location
/// whose normalized, language-family-prefixed token sequence is identical.
/// `locations` is kept in canonical `(path, start line)` order, so
/// `locations[0]` is the group's "first occurrence".
#[derive(Debug, Clone, PartialEq)]
pub struct CloneGroup {
    pub language: LanguageFamily,
    pub locations: Vec<CloneLocation>,
    /// D14: `Σ source_lines` over every location but the first — the
    /// ranking metric ("redundant lines beyond the first occurrence").
    pub redundant_lines: usize,
}

/// Everything the clones stage (WS-3) computed: every clone group, ranked
/// by `redundant_lines` descending.
#[derive(Debug, Clone, PartialEq)]
pub struct ClonesResult {
    pub groups: Vec<CloneGroup>,
}

/// A stable identifier for one of the six v1 wasteful-code rules (D22),
/// e.g. `"JAVA-EMPTY-CATCH"`. Every rule id is documented in
/// `docs/wasteful-rules.md`.
pub type RuleId = &'static str;

/// One rule hit (D22): the rule id plus the source lines its flagged AST
/// region spans. `flagged_lines` is every D11-counted line of the
/// statements the rule actually flagged (a lone closing brace does not
/// count) — for `*-UNREACHABLE-AFTER-RETURN` in JS/TS that excludes an
/// exempt statement's lines even when they sit inside the region — while
/// `start_line`/`end_line` keep the region's full extent for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFinding {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub rule_id: RuleId,
    pub start_line: usize,
    pub end_line: usize,
    pub flagged_lines: Vec<usize>,
}

/// D12/D20/D11: one scanned file's language family and D12 scanned-line
/// count, joined for the per-family verbosity denominator — `FileScanSummary`
/// alone carries no language — plus the D11-filtered set of executable
/// source lines across the whole file. `compute_verbosity` uses
/// `executable_lines` to filter a clone occurrence's `[start_line,
/// end_line]` span before it enters the numerator, so a blank, comment-only
/// or brace-only line inside that span does not inflate verbosity, the same
/// filter a rule finding's own `flagged_lines` already gets at detection
/// time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLanguageLines {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub scanned_lines: usize,
    pub executable_lines: BTreeSet<usize>,
}

/// D23: one verbosity fraction — distinct flagged lines over scanned
/// lines, `0.0` when `scanned_lines` is `0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerbosityScore {
    pub flagged_lines: usize,
    pub scanned_lines: usize,
    pub ratio: f64,
}

/// The three D23 verbosity scores a scan produces: overall, and each
/// language family (D20) computed separately.
#[derive(Debug, Clone, PartialEq)]
pub struct VerbosityScores {
    pub overall: VerbosityScore,
    pub java: VerbosityScore,
    pub js_ts: VerbosityScore,
}

/// Everything the rules stage (WS-4) computed: every rule finding (D22)
/// plus the D23 verbosity score. `incomplete` mirrors
/// `MetricsResult.incomplete` (D18): at least one file failed to parse, so
/// both are a partial view.
#[derive(Debug, Clone, PartialEq)]
pub struct RulesResult {
    pub findings: Vec<RuleFinding>,
    pub verbosity: VerbosityScores,
    pub incomplete: bool,
}

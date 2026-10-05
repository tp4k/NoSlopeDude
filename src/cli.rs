//! The `scan` and `check` command-line surface.

use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};

use crate::model::DEFAULT_MIN_CLONE_LINES;

#[derive(Debug, Parser)]
#[command(name = "nsd", about = "Code quality scanner for Java and JS/TS")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scan a local folder or a public GitHub repository.
    Scan(ScanArgs),
    /// Check a candidate change against its base and exit non-zero on regressions.
    Check(CheckArgs),
}

/// How `check` prints its diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One escaped line per diagnostic.
    Terminal,
    /// One canonical JSON document.
    Json,
}

#[derive(Debug, Parser, Clone, PartialEq, Eq)]
#[command(group(ArgGroup::new("candidate").required(true).args(["staged", "base"])))]
pub struct CheckArgs {
    /// Check the Git index against `HEAD`.
    #[arg(long)]
    pub staged: bool,

    /// Check `HEAD` against its merge base with this reference.
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,

    /// Check the working tree instead of `HEAD` (requires --base).
    #[arg(long, conflicts_with = "staged")]
    pub worktree: bool,

    /// Trusted configuration file outside the checkout, replacing nsd.yml.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Output format of the diagnostics.
    #[arg(long, value_enum, default_value_t = Format::Terminal)]
    pub format: Format,
}

#[derive(Debug, Parser, Clone, PartialEq, Eq)]
pub struct ScanArgs {
    /// Local folder path or public GitHub URL to scan.
    pub target: String,

    /// Directory to write report.json and report.html into (created if missing).
    #[arg(long)]
    pub output: PathBuf,

    /// Include test files in the scan (excluded by default).
    #[arg(long, default_value_t = false)]
    pub include_tests: bool,

    /// Glob of paths to exclude from the scan; may be repeated.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Minimum number of duplicated source lines to report as a clone.
    #[arg(long, default_value_t = DEFAULT_MIN_CLONE_LINES)]
    pub min_clone_lines: u32,
}

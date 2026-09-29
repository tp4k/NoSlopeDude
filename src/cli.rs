//! The `scan` command-line surface.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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

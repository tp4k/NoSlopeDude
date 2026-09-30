//! The complexity-policy diagnostic record (M3-1): code, candidate callable
//! and, when paired, its base callable. Carries no severity; the exit-code
//! mapping is M5-3's job.

use crate::git::path::RepoPath;

pub const CODE_E101: &str = "NSD-E101";
pub const CODE_E102: &str = "NSD-E102";
pub const CODE_G102: &str = "NSD-G102";

/// The matched base callable behind a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseCallable {
    pub path: RepoPath,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
}

/// One E101, E102 or G102 finding about a candidate callable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDiagnostic {
    pub code: &'static str,
    pub candidate_path: RepoPath,
    pub candidate_name: String,
    pub candidate_start_line: usize,
    pub candidate_end_line: usize,
    pub base: Option<BaseCallable>,
}

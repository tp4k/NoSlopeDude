//! The path-independent payload of one cache entry.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::analysis::{AnalyzedCallable, AnalyzedFinding, FileAnalysis, UnanalyzableReason};
use crate::clones::Candidate;
use crate::model::LanguageFamily;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedAnalysis {
    Analyzed(Box<AnalyzedPayload>),
    Unanalyzable(CachedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedReason {
    InvalidEncoding,
    ParserUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedPayload {
    pub damage: Vec<CachedDamage>,
    pub callables: Vec<CachedCallable>,
    pub findings: Vec<CachedFinding>,
    pub executable_lines: Vec<usize>,
    pub clone_candidates: Vec<CachedCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDamage {
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedIdentity {
    pub owner_digest: String,
    pub kind: String,
    pub name: String,
    pub signature: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCallable {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
    pub identity: CachedIdentity,
    pub body_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedFinding {
    pub rule_id: String,
    pub start_line: usize,
    pub end_line: usize,
    pub flagged_lines: Vec<usize>,
    pub syntax_digest: String,
    pub enclosing_callable: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCandidate {
    pub key: String,
    pub container: u32,
    pub first_statement: u32,
    pub start_line: usize,
    pub end_line: usize,
    pub source_lines: usize,
    pub statement_count: usize,
}

/// A hit rebuilt for one path.
#[derive(Debug, Clone, PartialEq)]
pub struct Hydrated {
    pub callables: Vec<AnalyzedCallable>,
    pub findings: Vec<AnalyzedFinding>,
    pub damage: Vec<CachedDamage>,
    pub executable_lines: Vec<usize>,
}

impl CachedAnalysis {
    pub fn from_analysis(analysis: &FileAnalysis, source: &str, min_clone_lines: u32) -> Self {
        let _ = (analysis, source, min_clone_lines);
        todo!()
    }

    pub fn unanalyzable(reason: UnanalyzableReason) -> Option<Self> {
        let _ = reason;
        todo!()
    }

    pub fn unanalyzable_reason(&self) -> Option<UnanalyzableReason> {
        todo!()
    }

    pub fn hydrate(
        &self,
        path: &Path,
        language: LanguageFamily,
    ) -> Result<Hydrated, UnanalyzableReason> {
        let _ = (path, language);
        todo!()
    }

    pub(crate) fn clone_candidates(&self, file_index: u32) -> Vec<(u128, Candidate)> {
        let _ = file_index;
        todo!()
    }
}

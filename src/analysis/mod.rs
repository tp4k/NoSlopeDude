//! Per-file snapshot analysis: one file's bytes and repo path in, its
//! callable metrics, identities, fingerprints and rule findings out.

use std::path::{Path, PathBuf};

use crate::identity::CallableIdentity;
use crate::lower::IrFile;
use crate::model::{Callable, LanguageFamily, RuleFinding};

/// Why a file could not be analyzed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnanalyzableReason {
    NonUtf8Path,
    UnsupportedExtension,
    TooLarge,
    InvalidEncoding,
    ParserUnavailable,
}

/// One callable's metrics, identity and body fingerprint.
pub struct AnalyzedCallable {
    pub metrics: Callable,
    pub identity: CallableIdentity,
    pub body_fingerprint: String,
}

/// One rule finding plus its normalized-syntax digest and enclosing callable.
pub struct AnalyzedFinding {
    pub finding: RuleFinding,
    pub syntax_digest: String,
    pub enclosing_callable: Option<usize>,
}

/// The in-memory analysis of one file.
pub struct FileAnalysis {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub ir: IrFile,
    pub callables: Vec<AnalyzedCallable>,
    pub findings: Vec<AnalyzedFinding>,
}

/// Analyzes one file from its repo path and bytes.
pub fn analyze_file(
    _relative_path: &Path,
    _bytes: &[u8],
) -> Result<FileAnalysis, UnanalyzableReason> {
    unimplemented!("analyze_file")
}

//! Stage 2 (WS-2): tree-sitter parsing. Parses every discovered file with
//! the pinned grammar for its extension (D20). Each file is parsed with a
//! fresh `Parser`, so per-file parallelism (D21) never shares a `Parser`
//! across threads — `Parser::parse` takes `&mut self`.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tree_sitter::{Language, Parser, Tree};

use crate::model::{DiscoveredFile, Grammar, LanguageFamily, ParseFailure, ParseFailureReason};

/// A file successfully parsed into a tree-sitter tree, plus the source
/// text the tree's byte ranges index into.
pub struct ParsedFile {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub source: String,
    pub tree: Tree,
}

/// Parses every file in `files` (relative to `root`), in parallel with one
/// `Parser` per task (D21). A file that cannot be read, whose extension is
/// not one of the seven scanned extensions, or that tree-sitter reports a
/// syntax error for, is returned as a `ParseFailure` instead (D18): the
/// scan continues past it rather than failing.
pub fn parse_all(root: &Path, files: &[DiscoveredFile]) -> (Vec<ParsedFile>, Vec<ParseFailure>) {
    let results: Vec<Result<ParsedFile, ParseFailure>> =
        files.par_iter().map(|file| parse_one(root, file)).collect();

    let mut parsed = Vec::with_capacity(results.len());
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(file) => parsed.push(file),
            Err(failure) => failures.push(failure),
        }
    }
    (parsed, failures)
}

fn parse_one(root: &Path, file: &DiscoveredFile) -> Result<ParsedFile, ParseFailure> {
    let full_path = root.join(&file.relative_path);
    let source = std::fs::read_to_string(&full_path).map_err(|error| ParseFailure {
        relative_path: file.relative_path.clone(),
        reason: ParseFailureReason::Unreadable,
        detail: Some(format!("cannot read file: {error}")),
    })?;

    let extension = file
        .relative_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let grammar = Grammar::for_extension(extension).ok_or_else(|| ParseFailure {
        relative_path: file.relative_path.clone(),
        reason: ParseFailureReason::UnsupportedExtension,
        detail: Some(format!("unrecognized extension {extension:?}")),
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&language_for(grammar))
        .map_err(|error| ParseFailure {
            relative_path: file.relative_path.clone(),
            reason: ParseFailureReason::GrammarSetup,
            detail: Some(format!("failed to set grammar: {error}")),
        })?;

    let tree = parser.parse(&source, None).ok_or_else(|| ParseFailure {
        relative_path: file.relative_path.clone(),
        reason: ParseFailureReason::GrammarSetup,
        detail: Some("tree-sitter returned no tree".to_string()),
    })?;

    if tree.root_node().has_error() {
        return Err(ParseFailure {
            relative_path: file.relative_path.clone(),
            reason: ParseFailureReason::SyntaxError,
            detail: None,
        });
    }

    Ok(ParsedFile {
        relative_path: file.relative_path.clone(),
        language: file.language,
        source,
        tree,
    })
}

fn language_for(grammar: Grammar) -> Language {
    match grammar {
        Grammar::Java => tree_sitter_java::LANGUAGE.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

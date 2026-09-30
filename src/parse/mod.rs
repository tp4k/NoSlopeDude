//! Stage 2 (WS-2): tree-sitter parsing. Parses every discovered file with
//! the pinned grammar for its extension (D20). Each file is parsed with a
//! fresh `Parser`, so per-file parallelism (D21) never shares a `Parser`
//! across threads — `Parser::parse` takes `&mut self`.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tree_sitter::{Language, Node, Parser, Tree};

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
/// not one of the seven scanned extensions, or that tree-sitter could not
/// build a tree for at all, produces no `ParsedFile` and one `ParseFailure`
/// (D18): the scan continues past it rather than failing. A file
/// tree-sitter *did* build a tree for but reports a syntax error on (WS-6
/// salvage) produces both: a `ParsedFile` (so the lowering/analyzer stages
/// still see it -- IR-level typed damage spans drive fail-closed exclusion
/// at entity granularity from there, not this whole-file granularity) and a
/// `ParseFailure{reason: SyntaxError}` alongside it, so `parse_failures`
/// stays non-empty -- and `pipeline.rs`'s own untouched `!parse_failures
/// .is_empty()` plumbing keeps marking the scan incomplete -- for exactly
/// as long as the file carries residual damage.
pub fn parse_all(root: &Path, files: &[DiscoveredFile]) -> (Vec<ParsedFile>, Vec<ParseFailure>) {
    let results: Vec<(Option<ParsedFile>, Option<ParseFailure>)> =
        files.par_iter().map(|file| parse_one(root, file)).collect();

    let mut parsed = Vec::with_capacity(results.len());
    let mut failures = Vec::new();
    for (file, failure) in results {
        if let Some(file) = file {
            parsed.push(file);
        }
        if let Some(failure) = failure {
            failures.push(failure);
        }
    }
    (parsed, failures)
}

fn parse_one(root: &Path, file: &DiscoveredFile) -> (Option<ParsedFile>, Option<ParseFailure>) {
    let full_path = root.join(&file.relative_path);
    let source = match std::fs::read_to_string(&full_path) {
        Ok(source) => source,
        Err(error) => {
            return (
                None,
                Some(ParseFailure {
                    relative_path: file.relative_path.clone(),
                    reason: ParseFailureReason::Unreadable,
                    detail: Some(format!("cannot read file: {error}")),
                }),
            );
        }
    };

    parse_source(&file.relative_path, file.language, source)
}

/// `parse_one`'s post-read half, also the entry point for callers that
/// already hold a file's text (a snapshot's bytes, not a worktree path).
pub(crate) fn parse_source(
    relative_path: &Path,
    language: LanguageFamily,
    source: String,
) -> (Option<ParsedFile>, Option<ParseFailure>) {
    let extension = relative_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let Some(grammar) = Grammar::for_extension(extension) else {
        return (
            None,
            Some(ParseFailure {
                relative_path: relative_path.to_path_buf(),
                reason: ParseFailureReason::UnsupportedExtension,
                detail: Some(format!("unrecognized extension {extension:?}")),
            }),
        );
    };

    let mut parser = Parser::new();
    if let Err(error) = parser.set_language(&language_for(grammar)) {
        return (
            None,
            Some(ParseFailure {
                relative_path: relative_path.to_path_buf(),
                reason: ParseFailureReason::GrammarSetup,
                detail: Some(format!("failed to set grammar: {error}")),
            }),
        );
    }

    let Some(tree) = parser.parse(&source, None) else {
        return (
            None,
            Some(ParseFailure {
                relative_path: relative_path.to_path_buf(),
                reason: ParseFailureReason::GrammarSetup,
                detail: Some("tree-sitter returned no tree".to_string()),
            }),
        );
    };

    let has_error = tree.root_node().has_error();
    let parsed_file = ParsedFile {
        relative_path: relative_path.to_path_buf(),
        language,
        source,
        tree,
    };

    if has_error {
        let failure = ParseFailure {
            relative_path: relative_path.to_path_buf(),
            reason: ParseFailureReason::SyntaxError,
            detail: first_error_line(parsed_file.tree.root_node())
                .map(|line| format!("first error at line {line}")),
        };
        return (Some(parsed_file), Some(failure));
    }

    (Some(parsed_file), None)
}

/// The 1-based line of the first `ERROR` or `MISSING` node in document
/// order, descending only into subtrees that `has_error`.
fn first_error_line(root: Node<'_>) -> Option<usize> {
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            return Some(node.start_position().row + 1);
        }
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return None;
            }
        }
    }
}

fn language_for(grammar: Grammar) -> Language {
    match grammar {
        Grammar::Java => tree_sitter_java_orchard::LANGUAGE.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

//! Per-file snapshot analysis: one file's bytes and repo path in, its
//! callable metrics, identities, fingerprints and rule findings out. The
//! file is parsed and lowered exactly once; metrics, rules and identity all
//! read that one `IrFile`. Unlike `parse_all`, it reads nothing from disk, so
//! a snapshot's bytes (a commit or the index) analyze the same as a worktree
//! file.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use crate::clones;
use crate::git::snapshot::SOURCE_CEILING_BYTES;
use crate::hashing::Digest;
use crate::identity::{self, CallableIdentity};
use crate::ir::IrNode;
use crate::lower::{self, IrFile};
use crate::metrics;
use crate::model::{Callable, LanguageFamily, RuleFinding};
use crate::parse;
use crate::rules;

/// Hash domain of a finding's normalized-syntax digest, distinct from the
/// callable body-fingerprint and clone-run domains.
const FINDING_SYNTAX_FAMILY_PREFIX: &str = "finding-syntax";

/// Why a file could not be analyzed; `NSD-A102` reports all but
/// `UnsupportedExtension`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnanalyzableReason {
    NonUtf8Path,
    UnsupportedExtension,
    TooLarge,
    InvalidEncoding,
    ParserUnavailable,
}

impl UnanalyzableReason {
    /// A stable snake_case spelling of this reason.
    pub fn label(self) -> &'static str {
        match self {
            UnanalyzableReason::NonUtf8Path => "non_utf8_path",
            UnanalyzableReason::UnsupportedExtension => "unsupported_extension",
            UnanalyzableReason::TooLarge => "too_large",
            UnanalyzableReason::InvalidEncoding => "invalid_encoding",
            UnanalyzableReason::ParserUnavailable => "parser_unavailable",
        }
    }
}

/// One callable's metrics, identity and body fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzedCallable {
    pub metrics: Callable,
    pub identity: CallableIdentity,
    pub body_fingerprint: String,
}

/// One rule finding with a whitespace- and comment-insensitive digest of the
/// IR subtrees it flagged, and the index (into `FileAnalysis::callables`) of
/// its innermost enclosing callable, `None` for file-level code.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzedFinding {
    pub finding: RuleFinding,
    pub syntax_digest: String,
    pub enclosing_callable: Option<usize>,
}

/// The in-memory analysis of one file. `callables` is index-aligned with
/// `ir.callables`; `ir.damage` holds the salvaged parse-damage spans.
pub struct FileAnalysis {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub ir: IrFile,
    pub callables: Vec<AnalyzedCallable>,
    pub findings: Vec<AnalyzedFinding>,
}

/// Analyzes one file from its repo path and bytes. Checks run in the order
/// extension, path, size, encoding, so a skippable binary never fails closed.
pub fn analyze_file(
    relative_path: &Path,
    bytes: &[u8],
) -> Result<FileAnalysis, UnanalyzableReason> {
    let extension = relative_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let language = LanguageFamily::from_extension(extension)
        .ok_or(UnanalyzableReason::UnsupportedExtension)?;
    relative_path
        .to_str()
        .ok_or(UnanalyzableReason::NonUtf8Path)?;
    if bytes.len() as u64 > SOURCE_CEILING_BYTES {
        return Err(UnanalyzableReason::TooLarge);
    }
    let source =
        String::from_utf8(bytes.to_vec()).map_err(|_| UnanalyzableReason::InvalidEncoding)?;

    let (parsed, _syntax_error) = parse::parse_source(relative_path, language, source);
    let parsed = parsed.ok_or(UnanalyzableReason::ParserUnavailable)?;
    let ir = lower::lower_file(&parsed);

    let metrics = metrics::callables_from_ir(&parsed.relative_path, language, &ir);
    let identities = identity::identities(&ir);
    let callables: Vec<AnalyzedCallable> = metrics
        .into_iter()
        .zip(identities)
        .zip(&ir.callables)
        .map(|((metrics, identity), ir_callable)| AnalyzedCallable {
            metrics,
            identity,
            body_fingerprint: identity::body_fingerprint(&ir, ir_callable, &parsed.source),
        })
        .collect();

    let callable_index = CallableIndex::of(&ir);
    let mut flagged = rules::findings_with_nodes(&parsed, &ir);
    flagged.sort_by(|(a, _), (b, _)| rules::cmp_findings(a, b));
    let findings = flagged
        .into_iter()
        .map(|(finding, nodes)| AnalyzedFinding {
            syntax_digest: syntax_digest(&nodes, &parsed.source),
            enclosing_callable: nodes
                .first()
                .and_then(|node| callable_index.innermost(node)),
            finding,
        })
        .collect();

    Ok(FileAnalysis {
        relative_path: parsed.relative_path,
        language,
        ir,
        callables,
        findings,
    })
}

/// A digest over the normalized leaf tokens of each flagged subtree
/// (`clones::ir_statement_tokens`: comments dropped, anonymous-token
/// whitespace collapsed), so reformatting never changes it.
fn syntax_digest(nodes: &[&IrNode], source: &str) -> String {
    let mut digest = Digest::new(FINDING_SYNTAX_FAMILY_PREFIX);
    for node in nodes {
        digest.push(clones::ir_statement_tokens(node, source).as_bytes());
    }
    format!("blake3:{:032x}", digest.finish())
}

/// Callables ordered by start (then longest first, then document order), each
/// with its enclosing entry. Callable spans nest or are disjoint, so the
/// innermost callable containing a node is an ancestor-or-self of the last
/// entry starting at or before it.
struct CallableIndex {
    entries: Vec<IndexEntry>,
}

struct IndexEntry {
    end: u32,
    start: u32,
    callable: usize,
    parent: Option<usize>,
}

impl CallableIndex {
    fn of(ir: &IrFile) -> CallableIndex {
        let mut order: Vec<usize> = (0..ir.callables.len()).collect();
        order.sort_by_key(|&index| {
            let span = ir.callables[index].span;
            (span.start_byte, Reverse(span.end_byte), index)
        });
        let mut entries: Vec<IndexEntry> = Vec::with_capacity(order.len());
        let mut open: Vec<usize> = Vec::new();
        for callable in order {
            let span = ir.callables[callable].span;
            while open
                .last()
                .is_some_and(|&top| entries[top].end < span.end_byte)
            {
                open.pop();
            }
            entries.push(IndexEntry {
                end: span.end_byte,
                start: span.start_byte,
                callable,
                parent: open.last().copied(),
            });
            open.push(entries.len() - 1);
        }
        CallableIndex { entries }
    }

    /// The index of the smallest callable whose span contains `node`; among
    /// callables with equal spans, the later one in document order.
    fn innermost(&self, node: &IrNode) -> Option<usize> {
        let preceding = self
            .entries
            .partition_point(|entry| entry.start <= node.span.start_byte);
        let mut cursor = preceding.checked_sub(1);
        while let Some(position) = cursor {
            let entry = &self.entries[position];
            if node.span.end_byte <= entry.end {
                return Some(entry.callable);
            }
            cursor = entry.parent;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CallableKind, IrCallable, Span};

    const LINE: u32 = 1;

    fn span(start_byte: u32, end_byte: u32) -> Span {
        Span {
            start_byte,
            end_byte,
            start_line: LINE,
            end_line: LINE,
        }
    }

    fn callable(callable_span: Span) -> IrCallable {
        IrCallable {
            span: callable_span,
            body_span: callable_span,
            name: String::new(),
            kind: CallableKind::JavaMethod,
            is_anonymous: false,
            signature: Vec::new(),
            owner: None,
        }
    }

    fn ir_with(callable_spans: &[Span]) -> IrFile {
        IrFile {
            relative_path: PathBuf::from("Synthetic.java"),
            language: LanguageFamily::Java,
            root: IrNode::empty(span(0, 0)),
            damage: Vec::new(),
            callables: callable_spans.iter().copied().map(callable).collect(),
            blocks: Vec::new(),
            owners: Vec::new(),
            excluded_callables: Vec::new(),
            excluded_blocks: Vec::new(),
        }
    }

    #[test]
    fn callable_index_prefers_the_later_of_equal_spans() {
        let shared = span(0, 50);
        let index = CallableIndex::of(&ir_with(&[shared, shared]));

        assert_eq!(index.innermost(&IrNode::empty(span(10, 20))), Some(1));
    }

    #[test]
    fn callable_index_orders_a_longer_span_before_a_shorter_one_at_the_same_start() {
        let inner = span(0, 50);
        let outer = span(0, 100);
        let index = CallableIndex::of(&ir_with(&[inner, outer]));

        assert_eq!(index.innermost(&IrNode::empty(span(10, 20))), Some(0));
        assert_eq!(index.innermost(&IrNode::empty(span(60, 70))), Some(1));
    }
}

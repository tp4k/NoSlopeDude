//! Directive discovery: `nsd-ignore` comments found on the IR's comment
//! leaves, never by scanning text, so a string literal is never a directive.

use crate::ir::IrNode;
use crate::model::RuleId;
use crate::rules::ALL_RULE_IDS;

const DIRECTIVE_PREFIX: &str = "nsd-ignore";
const LINE_COMMENT_OPENER: &str = "//";
const BLOCK_COMMENT_OPENER: &str = "/*";
const BLOCK_DOC_MARK: char = '*';
const RULE_OPEN: char = '[';
const RULE_CLOSE: char = ']';
const REASON_SEPARATOR: char = ':';

/// A valid directive names one of `ALL_RULE_IDS`; every other `nsd-ignore`
/// comment is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Form {
    Valid(RuleId),
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Directive {
    pub line: usize,
    pub text: String,
    pub form: Form,
}

/// Every `nsd-ignore` comment of one file, sorted by line.
pub(super) fn directives(root: &IrNode, source: &[u8]) -> Vec<Directive> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        stack.extend(&node.children);
        if node.is_comment {
            found.extend(directive(node, source));
        }
    }
    found.sort_by_key(|directive| directive.line);
    found
}

fn directive(node: &IrNode, source: &[u8]) -> Option<Directive> {
    let start = node.span.start_byte as usize;
    let bytes = source.get(start..node.span.end_byte as usize)?;
    let text = String::from_utf8_lossy(bytes);
    let (opener, body) = if let Some(body) = text.strip_prefix(LINE_COMMENT_OPENER) {
        (LINE_COMMENT_OPENER, body)
    } else {
        let body = text.strip_prefix(BLOCK_COMMENT_OPENER)?;
        (
            BLOCK_COMMENT_OPENER,
            body.trim_start_matches(BLOCK_DOC_MARK),
        )
    };
    let rest = body.trim_start().strip_prefix(DIRECTIVE_PREFIX)?;
    let valid = opener == LINE_COMMENT_OPENER && is_standalone(source, start);
    let form = match parse_rule(rest) {
        Some(rule) if valid => Form::Valid(rule),
        _ => Form::Invalid,
    };
    Some(Directive {
        line: node.span.start_line as usize,
        text: text.into_owned(),
        form,
    })
}

/// `[RULE]: reason` with a known rule and a non-empty reason.
fn parse_rule(rest: &str) -> Option<RuleId> {
    let inner = rest.strip_prefix(RULE_OPEN)?;
    let (rule, after) = inner.split_once(RULE_CLOSE)?;
    let reason = after.strip_prefix(REASON_SEPARATOR)?;
    if reason.trim().is_empty() {
        return None;
    }
    ALL_RULE_IDS.iter().copied().find(|id| *id == rule)
}

fn is_standalone(source: &[u8], start: usize) -> bool {
    let before = &source[..start];
    let line_start = before
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |position| position + 1);
    before[line_start..].iter().all(u8::is_ascii_whitespace)
}

//! M1-7: a line-independent callable identity, computed from the lowering's
//! own facts rather than a grammar string or a byte/line offset
//! (`nsd-plan-final.md` *M1-M2*: "Replace `<anonymous>@<line>` with a
//! line-independent identity; the current form makes every insertion above
//! an anonymous callable look like delete+add"). Takes an already-lowered
//! `&lower::IrFile` plus the file's own source text; it never re-parses and
//! never re-lowers.
//!
//! `CallableIdentity` is owner chain + kind + name (`<anonymous>` when there
//! is no declared name) + signature -- no ordinal (correction #13,
//! `nsd-plan-final.md` *Stable data model*), path-free, and carrying no line
//! or byte offset. It is deliberately **not** unique within a file:
//! same-key siblings (e.g. two anonymous callbacks in the same owner) share
//! one `CallableIdentity`; `IrFile.callables`' own document order
//! (`src/ir/mod.rs:122-123`) is what WS-3 falls back on, breaking same-key
//! ties by body fingerprint, then source order.
//!
//! The body fingerprint is a separate, per-callable `blake3:<32 lowercase
//! hex>` digest over the normalized leaf tokens of the `IrNode` subtree at
//! `body_span` -- `clones::ir_statement_tokens`'s own normalization (comment
//! leaves dropped, anonymous-leaf whitespace collapsed), reused directly
//! rather than re-derived, so "exact normalized body fingerprint" has one
//! definition in the crate (answer 5). Hashed under this module's own family
//! prefix plus the language family -- never `clones::mod.rs`'s own
//! per-language prefix (that is the clone-run hash domain: sharing it would
//! let a body hash collide with a clone-run hash) and never
//! `src/golden.rs`'s `BODY_FAMILY_PREFIX` ("body" there means the report
//! body).

use crate::clones;
use crate::hashing::Digest;
use crate::ir::{CallableKind, IrCallable, IrNode, OwnerSegment, Span};
use crate::lower::IrFile;
use crate::model::LanguageFamily;

/// The display-name sentinel `CallableIdentity::name` carries for an
/// undeclared callable -- deliberately not `IrCallable.name`'s own
/// `<anonymous>@<line>` fallback, which embeds the declaration's line number
/// and so is exactly the thing this module exists to stop using as identity.
const ANONYMOUS_NAME: &str = "<anonymous>";

/// This module's own BLAKE3 family-prefix domains (one per language family),
/// distinct from `clones::mod.rs`'s `"java"`/`"js_ts"` clone-run domain and
/// from `src/golden.rs`'s `"golden-digest-body"` report-body domain -- see
/// this module's own doc comment for why sharing either would be unsafe.
const BODY_FINGERPRINT_JAVA_FAMILY_PREFIX: &str = "callable-body-java";
const BODY_FINGERPRINT_JSTS_FAMILY_PREFIX: &str = "callable-body-js_ts";

/// A callable's line-independent structural identity: owner chain + kind +
/// name + signature. See this module's own doc comment for the no-ordinal,
/// path-free, non-unique-within-a-file guarantees.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallableIdentity {
    pub owner_chain: Vec<OwnerSegment>,
    pub kind: CallableKind,
    pub name: String,
    pub signature: Vec<String>,
}

/// One `IrCallable`'s `CallableIdentity`, derived entirely from facts the
/// lowering already recorded on it -- no re-parse, no re-lowering, and (per
/// this module's own doc comment) no reuse of `IrCallable.name` as identity:
/// `name` here is the `ANONYMOUS_NAME` sentinel when `callable.is_anonymous`,
/// never `callable.name`'s own line-embedding fallback.
///
/// Round 2 (WS-1 triage row 1): takes `ir_file` too, now that `callable.owner`
/// is only an index into `ir_file.owners` rather than a self-contained
/// `Vec<OwnerSegment>` -- `owner_chain` (below) walks that table to
/// materialize the chain this function returns.
pub fn callable_identity(ir_file: &IrFile, callable: &IrCallable) -> CallableIdentity {
    let name = if callable.is_anonymous {
        ANONYMOUS_NAME.to_string()
    } else {
        callable.name.clone()
    };
    CallableIdentity {
        owner_chain: owner_chain(ir_file, callable.owner),
        kind: callable.kind,
        name,
        signature: callable.signature.clone(),
    }
}

/// Every callable's `CallableIdentity` in `ir_file`, in `IrFile.callables`'
/// own document order (`src/ir/mod.rs:122-123`) -- the order WS-3 falls back
/// on to separate a same-key group, by body fingerprint then source order.
pub fn identities(ir_file: &IrFile) -> Vec<CallableIdentity> {
    ir_file
        .callables
        .iter()
        .map(|callable| callable_identity(ir_file, callable))
        .collect()
}

/// Materializes one callable's full owner chain, outermost first, by walking
/// `OwnerEntry::parent` links from `owner` (its innermost enclosing owner)
/// out to the root and reversing -- an iterative `while let`, not recursion,
/// so a 15,000-level nesting cannot overflow the stack (D18). `owner` is
/// `None` for a top-level callable with no lexical owner, giving an empty
/// chain.
fn owner_chain(ir_file: &IrFile, owner: Option<u32>) -> Vec<OwnerSegment> {
    let mut chain = Vec::new();
    let mut current = owner;
    while let Some(index) = current {
        let entry = &ir_file.owners[index as usize];
        chain.push(entry.segment.clone());
        current = entry.parent;
    }
    chain.reverse();
    chain
}

/// `callable`'s per-body fingerprint: `clones::ir_statement_tokens`'s own
/// normalized leaf-token stream (see this module's own doc comment) over the
/// `IrNode` subtree at `callable.body_span`, hashed under this module's own
/// language-specific family prefix. `source` must be the same source text
/// `ir_file` was lowered from.
pub fn body_fingerprint(ir_file: &IrFile, callable: &IrCallable, source: &str) -> String {
    let fallback;
    let subtree = match find_ir_subtree(&ir_file.root, callable.body_span) {
        Some(subtree) => subtree,
        // D18 degrade-don't-panic: no `IrNode` in the (possibly
        // salvage-redacted) tree matches this exact span. Mirrors
        // `metrics::fallback_ir_body`'s own fallback for the same
        // structural gap on the same kind of lookup.
        None => {
            fallback = IrNode::empty(callable.body_span);
            &fallback
        }
    };
    let tokens = clones::ir_statement_tokens(subtree, source);
    let mut digest = Digest::new(body_fingerprint_family_prefix(ir_file.language));
    digest.push(tokens.as_bytes());
    format!("blake3:{:032x}", digest.finish())
}

fn body_fingerprint_family_prefix(language: LanguageFamily) -> &'static str {
    match language {
        LanguageFamily::Java => BODY_FINGERPRINT_JAVA_FAMILY_PREFIX,
        LanguageFamily::JsTs => BODY_FINGERPRINT_JSTS_FAMILY_PREFIX,
    }
}

/// Locates the `IrNode` produced for the tree-sitter node whose span is
/// `target`, by descending from `root` through whichever child's span
/// contains it. A fresh copy of `metrics::find_ir_subtree`'s own algorithm
/// (private to a module this stream may not touch): valid since `build_ir`
/// (`src/lower/mod.rs`) produces one `IrNode` per tree-sitter node in the
/// same nested-span shape. A `while` loop, not recursion, so a deeply nested
/// callable body cannot overflow the stack (D18).
fn find_ir_subtree(root: &IrNode, target: Span) -> Option<&IrNode> {
    let mut current = root;
    while current.span != target {
        current = current.children.iter().find(|child| {
            child.span.start_byte <= target.start_byte && target.end_byte <= child.span.end_byte
        })?;
    }
    Some(current)
}

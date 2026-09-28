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
//!
//! A5: `CallableIdentity`'s own owner-chain field is a fixed-size `Copy`
//! `OwnerDigest` -- one chained BLAKE3 digest per `ir_file.owners` entry,
//! computed exactly once per file in `owner_digests` (O(owners) total,
//! index order, parent-before-child, since `src/lower/mod.rs` always pushes
//! an owner entry after its own parent) -- rather than a `Vec<OwnerSegment>`
//! deep-copied per callable, which cost O(owners) *per callable* (an
//! `identities()` call on a chain nested `n` deep used to copy on the order
//! of `n^2` segments). Equality is unaffected: two owner chains hash equal
//! iff every segment in the two chains is equal in order, since each digest
//! folds in its own parent's digest, its own `OwnerKind`, and its own
//! optional name, under a family prefix distinct from the body-fingerprint
//! domains above. `owner_chain` (below) still materializes the full segment
//! list, for display and for tests that need to inspect it -- the compact
//! digest is deliberately not reversible.

use crate::clones;
use crate::hashing::Digest;
use crate::ir::{CallableKind, IrCallable, IrNode, OwnerKind, OwnerSegment, Span};
use crate::lower::IrFile;
use crate::model::LanguageFamily;

/// M2-3: matches callables across two snapshots by structural identity, git
/// rename, then exact body fingerprint. See the module's own doc comment.
pub mod matching;

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

/// A5's own hash domain for `OwnerDigest` -- distinct from both
/// body-fingerprint family prefixes above (and from `clones::mod.rs`'s and
/// `src/golden.rs`'s domains), so an owner-chain digest can never collide
/// with a body-fingerprint or clone-run hash even on identical input bytes.
const OWNER_DIGEST_FAMILY_PREFIX: &str = "callable-owner-chain";

/// A byte tag for each `OwnerKind`, folded into `hash_owner_entry` so two
/// segments with the same name but a different kind never hash equal.
fn owner_kind_tag(kind: OwnerKind) -> u8 {
    match kind {
        OwnerKind::NamedType => 0,
        OwnerKind::AnonymousClassBody => 1,
        OwnerKind::Namespace => 2,
        OwnerKind::Callable => 3,
    }
}

/// A5: a fixed-size, `Copy` chained-BLAKE3 digest standing in for a
/// callable's full owner chain -- see this module's own doc comment. Two
/// owner chains hash equal iff they carry the same segments in the same
/// order; `Default` is the empty chain (a top-level callable with no
/// lexical owner), also used as the "no parent" root case inside
/// `owner_digests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OwnerDigest(u128);

/// A callable's line-independent structural identity: owner chain + kind +
/// name + signature. See this module's own doc comment for the no-ordinal,
/// path-free, non-unique-within-a-file guarantees, and for A5's own
/// `OwnerDigest` design.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallableIdentity {
    pub owner_digest: OwnerDigest,
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
/// A5: recomputes the whole file's owner-digest table on every call -- fine
/// for this single-callable convenience API (its only production caller is
/// `identities()` itself, below, which instead computes the table once and
/// reuses it per callable in O(1)).
pub fn callable_identity(ir_file: &IrFile, callable: &IrCallable) -> CallableIdentity {
    let digests = owner_digests(ir_file);
    build_callable_identity(callable, &digests)
}

/// Every callable's `CallableIdentity` in `ir_file`, in `IrFile.callables`'
/// own document order (`src/ir/mod.rs:122-123`) -- the order WS-3 falls back
/// on to separate a same-key group, by body fingerprint then source order.
///
/// A5: computes `ir_file.owners`'s own digest table exactly once (O(owners)
/// total), then looks each callable's own digest up in O(1) -- the hot-path
/// fix this module's own doc comment describes, in place of the previous
/// per-callable full-chain walk-and-clone.
pub fn identities(ir_file: &IrFile) -> Vec<CallableIdentity> {
    let digests = owner_digests(ir_file);
    ir_file
        .callables
        .iter()
        .map(|callable| build_callable_identity(callable, &digests))
        .collect()
}

/// Shared by `callable_identity` and `identities`: builds one
/// `CallableIdentity` from an already-computed owner-digest table (`digests`,
/// index-aligned with `ir_file.owners`) rather than walking the owner table
/// itself.
fn build_callable_identity(callable: &IrCallable, digests: &[OwnerDigest]) -> CallableIdentity {
    let name = if callable.is_anonymous {
        ANONYMOUS_NAME.to_string()
    } else {
        callable.name.clone()
    };
    let owner_digest = match callable.owner {
        Some(index) => digests[index as usize],
        None => OwnerDigest::default(),
    };
    CallableIdentity {
        owner_digest,
        kind: callable.kind,
        name,
        signature: callable.signature.clone(),
    }
}

/// A5: one `OwnerDigest` per `ir_file.owners` entry, computed in index order
/// -- safe because `src/lower/mod.rs` always pushes an owner entry strictly
/// after its own parent entry (pre-order traversal), so `entry.parent`'s
/// digest is always already in `digests` by the time this loop reaches
/// `entry` itself. O(owners) total, each entry hashed exactly once
/// regardless of how many callables (or descendant owners) later share it.
fn owner_digests(ir_file: &IrFile) -> Vec<OwnerDigest> {
    let mut digests: Vec<OwnerDigest> = Vec::with_capacity(ir_file.owners.len());
    for entry in &ir_file.owners {
        let parent_digest = match entry.parent {
            Some(parent_index) => digests[parent_index as usize],
            None => OwnerDigest::default(),
        };
        digests.push(hash_owner_entry(parent_digest, &entry.segment));
    }
    digests
}

/// Chains one `OwnerSegment` onto its own parent's digest: hashes the
/// parent digest's own bytes, then the segment's `OwnerKind` tag, then its
/// optional name (a presence byte first, so `None` and `Some("")` can never
/// collide), under this module's own `OWNER_DIGEST_FAMILY_PREFIX`.
fn hash_owner_entry(parent_digest: OwnerDigest, segment: &OwnerSegment) -> OwnerDigest {
    let mut digest = Digest::new(OWNER_DIGEST_FAMILY_PREFIX);
    digest.push(&parent_digest.0.to_le_bytes());
    digest.push(&[owner_kind_tag(segment.kind)]);
    match &segment.name {
        Some(name) => {
            digest.push(&[1]);
            digest.push(name.as_bytes());
        }
        None => digest.push(&[0]),
    }
    OwnerDigest(digest.finish())
}

/// Materializes one callable's full owner chain, outermost first, by walking
/// `OwnerEntry::parent` links from `owner` (its innermost enclosing owner)
/// out to the root and reversing -- an iterative `while let`, not recursion,
/// so a 15,000-level nesting cannot overflow the stack (D18). `owner` is
/// `None` for a top-level callable with no lexical owner, giving an empty
/// chain.
///
/// A5: the only way to recover the segment list, now that
/// `CallableIdentity::owner_digest` is a compact, non-reversible hash --
/// used for display and by tests that need to inspect the actual chain
/// rather than just compare identities.
pub fn owner_chain(ir_file: &IrFile, owner: Option<u32>) -> Vec<OwnerSegment> {
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

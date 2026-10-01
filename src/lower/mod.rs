//! M0b: the two lowerings (`nsd-plan-final.md` M0b item 5) -- plain
//! functions over tree-sitter trees, no `Frontend` trait, no plugin system.
//! `src/lower/java.rs` and `src/lower/jsts.rs` hold every grammar-specific
//! `node.kind()` match (decision classification, terminator kinds, block/
//! catch-body structure, damage classification, clone-candidate statement
//! containers); this file holds only the traversal shared by both -- one
//! iterative `TreeCursor`-based tree build (D18: no per-AST-depth
//! recursion) over the raw tree-sitter tree, assembling the `IrNode` tree
//! itself as it goes. `metrics::walk_ir_excluding` is a separate,
//! later-stage traversal over the already-built `IrNode` tree -- an
//! explicit work-list, not a `TreeCursor`, so a different shape from this
//! one.

mod java;
mod jsts;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use tree_sitter::Node;

use crate::exec_lines::is_executable_leaf;
#[cfg(test)]
use crate::exec_lines::{for_each_descendant, is_comment_kind};
use crate::ir::{
    CallableKind, DamageKind, DamageSpan, DecisionKind, IrBlock, IrCallable, IrNode, OwnerEntry,
    OwnerSegment, Span, TerminatorKind,
};
use crate::model::LanguageFamily;
use crate::parse::ParsedFile;

/// M0c's fingerprint keys on this alongside `ir::IR_VERSION`; bump it
/// whenever the Java lowering's classification changes what an `IrNode`
/// carries for a Java file. Bumped 2 -> 3 for M0c-9: orchard's grammar
/// changes what a Java `ERROR`/`MISSING` node classifies as (`ir::DamageKind`'s
/// `JavaVarargsAnnotation` variant is gone).
pub const JAVA_LOWERING_VERSION: u32 = 3;

/// The JS/TS counterpart of `JAVA_LOWERING_VERSION`.
pub const JSTS_LOWERING_VERSION: u32 = 2;

/// One D14 separator between adjacent leaf tokens in a statement's
/// normalized stream -- the same control character
/// `clones::normalized_statement_tokens` uses as its own private
/// `TOKEN_SEPARATOR`. This lowering's `statement_token_stream` is an
/// independent re-derivation of that function (the floor prototype
/// `nsd-plan-final.md` M0b item 4 calls for), so the two must agree on this
/// exact value for the byte-for-byte comparison to mean anything; it cannot
/// be imported since the original is private to `src/clones/mod.rs`.
/// Test-only: nothing in `src/` reads a clone-candidate token stream yet
/// (`IrNode::token`, its only production reader, was removed as unread and
/// unbounded -- see `ir::IrNode`'s doc comment); this and the two helpers
/// below stay for WS-4 to build on, proven by the floor test in `mod tests`.
#[cfg(test)]
const STATEMENT_TOKEN_SEPARATOR: char = '\u{1}';

/// One lowered file: the IR tree, every typed damage span found while
/// building it, and the two per-file side tables (D8's callables, the
/// self-is-block predicate's blocks) -- accumulated the same way `damage`
/// is, so per-node memory does not grow to carry them.
pub struct IrFile {
    pub relative_path: PathBuf,
    pub language: LanguageFamily,
    pub root: IrNode,
    pub damage: Vec<DamageSpan>,
    pub callables: Vec<IrCallable>,
    pub blocks: Vec<IrBlock>,
    /// M1-7 round 2 (WS-1 triage row 1): the per-file owner table -- one
    /// `OwnerEntry` per named type, anonymous class body, namespace or
    /// callable, however many callables sit underneath it. `IrCallable::owner`
    /// indexes into this; `identity::callable_identity` walks it via
    /// `OwnerEntry::parent` to materialize a callable's own owner chain.
    ///
    /// A7 (WS-4): for a salvaged file, an entry whose contributing node lies
    /// inside a bare damage span, or inside an excluded callable's or
    /// block's span, is omitted from this table -- `OwnerEntry::parent` and
    /// `IrCallable::owner` are then remapped to index the compacted table,
    /// not the pre-pruning one.
    pub owners: Vec<OwnerEntry>,
    /// The spans of the callables salvage excluded, which are therefore
    /// absent from `callables`. A101 reads this to tell a damaged changed
    /// callable apart from a deleted one.
    pub excluded_callables: Vec<Span>,
    /// The spans of the blocks salvage excluded, which are therefore absent
    /// from `blocks`. A101 reads this for an edit inside an excluded block
    /// that no excluded callable contains.
    pub excluded_blocks: Vec<Span>,
}

/// WS-9 (C1): total number of `lower_file` calls made so far in this
/// process. Not `#[cfg(test)]`: `tests/ir_isolation.rs` links the
/// non-test library build, like every other integration test, so a
/// `#[cfg(test)]` counter would not be visible there.
static LOWERING_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Reads `LOWERING_COUNT`'s current value. `#[doc(hidden)]`: an
/// observability seam for `tests/ir_isolation.rs`, not public API.
#[doc(hidden)]
pub fn lowering_count() -> usize {
    LOWERING_COUNT.load(Ordering::Relaxed)
}

/// Lowers every parsed file, one rayon task per file (D21), matching the
/// upstream stages' own per-file parallelism.
pub fn lower_all(parsed_files: &[ParsedFile]) -> Vec<IrFile> {
    use rayon::prelude::*;
    parsed_files.par_iter().map(lower_file).collect()
}

/// Lowers one parsed file's whole tree into its `IrNode` root.
///
/// WS-6 salvage (*Architecture* -> salvage row): `parse::parse_one` no
/// longer rejects a whole file for `tree.root_node().has_error()`, so
/// `build_ir` above may run over a damaged tree. This function is the one
/// place that turns typed damage spans into fail-closed exclusions.
///
/// WS-6 round 3 redesign (security HIGH+MEDIUM+MEDIUM, perf HIGH+LOW, all
/// traced to this one function): `build_ir` already folds, for free during
/// its single tree walk, each callable's and block's own bottom-up "does
/// this entity's subtree touch damage anywhere" bit (`callable_dirty` /
/// `block_dirty`, mirroring the `catch_flags` inheritance it already does).
/// `cascade_exclusions` turns that into a final exclusion decision per
/// entity via one document-order interval-nesting sweep: an entity is
/// excluded if it is itself dirty *or* its nearest enclosing entity is
/// excluded -- the cascade a bottom-up fold alone cannot express, since a
/// damaged sibling region elsewhere in the same outer entity must still
/// exclude an inner clean entity regardless of whether the damage sits
/// before or after it in document order (this fixes (a): a clean callable
/// nested inside a damaged one no longer survives in `callables`/`blocks`
/// once its subtree is wiped). The rebuild target set is every bare damage
/// span (fixing (c): stray damage outside any callable/block is now always
/// redacted, not just entity-shaped damage) unioned with every excluded
/// entity's own span, held in a `HashSet` so `prune_damage`'s per-node
/// membership test is O(1) average instead of a linear `Vec` scan (fixing
/// (d): no per-node or per-entity work now scales with `damage.len()` or
/// the pruned-entity count). Each entity's own dirty bit is read exactly
/// once here (fixing (e): the old double damage-intersection evaluation
/// -- once to build the prune list, once again in `retain` -- is gone).
///
/// WS-9 (C1): increments `LOWERING_COUNT` on every call, the seam
/// `tests/ir_isolation.rs::test_pipeline_lowers_each_file_once` reads
/// through `lowering_count()` below to observe that `pipeline::run` lowers
/// each file exactly once.
pub fn lower_file(file: &ParsedFile) -> IrFile {
    LOWERING_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut damage = Vec::new();
    let mut callables = Vec::new();
    let mut blocks = Vec::new();
    let mut callable_dirty = Vec::new();
    let mut block_dirty = Vec::new();
    let mut owners = Vec::new();
    // A7 (WS-4): index-aligned with `owners`, this exact node's own span --
    // internal only, so the pruning below can test containment inside a
    // redaction target the same way `cascade_exclusions` already does for
    // `callables`/`blocks`. `OwnerEntry` itself carries no span (its public
    // shape is unchanged), so this table never leaves this function.
    let mut owner_spans = Vec::new();
    let mut tables = IrTables {
        damage: &mut damage,
        callables: &mut callables,
        blocks: &mut blocks,
        callable_dirty: &mut callable_dirty,
        block_dirty: &mut block_dirty,
        owners: &mut owners,
        owner_spans: &mut owner_spans,
    };
    let root = build_ir(
        file.tree.root_node(),
        file.language,
        &file.source,
        &mut tables,
    );

    let exclusions = cascade_exclusions(
        &damage,
        &callables,
        &callable_dirty,
        &blocks,
        &block_dirty,
        &owner_spans,
    );

    let mut redact_targets: HashSet<Span> = damage.iter().map(|entry| entry.span).collect();
    for (index, callable) in callables.iter().enumerate() {
        if exclusions.callables[index] {
            redact_targets.insert(callable.span);
        }
    }
    for (index, block) in blocks.iter().enumerate() {
        if exclusions.blocks[index] {
            redact_targets.insert(block.span);
        }
    }

    let mut kept_callables = Vec::with_capacity(callables.len());
    let mut excluded_callables = Vec::new();
    for (index, callable) in callables.into_iter().enumerate() {
        if exclusions.callables[index] {
            excluded_callables.push(callable.span);
        } else {
            kept_callables.push(callable);
        }
    }
    let mut kept_blocks = Vec::with_capacity(blocks.len());
    let mut excluded_blocks = Vec::new();
    for (index, block) in blocks.into_iter().enumerate() {
        if exclusions.blocks[index] {
            excluded_blocks.push(block.span);
        } else {
            kept_blocks.push(block);
        }
    }

    // A7 (WS-4): compact `owners` the same way, dropping every entry whose
    // own contributing node was inside a redaction target (`exclusions.
    // owners[index]`, computed by the same cascade sweep above), then remap
    // both `OwnerEntry::parent` and the surviving callables' own `owner`
    // through the old-to-new index map -- a kept callable's chain never
    // points into a redacted region (cascade already excludes it there too),
    // so every remap lookup below always resolves.
    let mut owner_remap: Vec<Option<u32>> = Vec::with_capacity(owners.len());
    let mut kept_owners = Vec::with_capacity(owners.len());
    for (index, owner) in owners.into_iter().enumerate() {
        if exclusions.owners[index] {
            owner_remap.push(None);
            continue;
        }
        owner_remap.push(Some(kept_owners.len() as u32));
        kept_owners.push(owner);
    }
    for owner in &mut kept_owners {
        owner.parent = owner
            .parent
            .and_then(|old_index| owner_remap[old_index as usize]);
    }
    for callable in &mut kept_callables {
        callable.owner = callable
            .owner
            .and_then(|old_index| owner_remap[old_index as usize]);
    }

    let root = prune_damage(root, &redact_targets);

    IrFile {
        relative_path: file.relative_path.clone(),
        language: file.language,
        root,
        damage,
        callables: kept_callables,
        blocks: kept_blocks,
        owners: kept_owners,
        excluded_callables,
        excluded_blocks,
    }
}

/// One (callable, block, owner) entity table's final inclusion decision,
/// aligned by index with the `callables`/`blocks`/`owners` slices
/// `cascade_exclusions` was given -- `true` means excluded (fail-closed:
/// dropped from the IR's side tables and, for `callables`/`blocks`, redacted
/// from the tree too). A7 (WS-4): `owners` carries no `own_dirty` bit of its
/// own (an owner-kind node's own damage-ness is never independently tracked,
/// unlike `callable_dirty`/`block_dirty`) -- an owner entry is excluded
/// purely by containment inside a redaction target (`ancestor_excluded`
/// below), the same "lies inside a bare damage span, or an excluded
/// callable's or block's span" test `lower_file`'s own doc comment
/// describes.
struct Exclusions {
    callables: Vec<bool>,
    blocks: Vec<bool>,
    owners: Vec<bool>,
}

/// Which entity table (and index into it) one sweep entry refers back to.
#[derive(Clone, Copy)]
enum EntityRef {
    Callable(usize),
    Block(usize),
    Owner(usize),
}

/// The cascade sweep `lower_file`'s own doc comment describes: callables,
/// blocks and bare damage spans are all just spans of the same tree, so
/// "does entity B sit inside excluded/damaged ancestor A" is answerable
/// purely from `(start_byte, end_byte)` ordering, with no dependency on the
/// tree's own node count. Sorts every entry into document order, outer
/// before inner (ascending `start_byte`; ties broken by descending
/// `end_byte`, so a wider outer span that opens at the same byte as an
/// inner one -- e.g. a callable whose single-statement body is itself the
/// next entity -- sorts first), then sweeps once with a stack of
/// currently-open ancestors: an entry closes the moment the sweep reaches a
/// span starting at or past its own `end_byte` (two entity spans, both
/// tree-sitter node spans, are always either nested or disjoint, never
/// partially overlapping, so this is exact, not a heuristic). `Θ(n log n)`
/// for the sort, `Θ(n)` for the sweep, `n = damage.len() + callables.len() +
/// blocks.len()`.
///
/// WS-6 round 4 (security HIGH regression from round 3's `35508d9`): a bare
/// damage span -- one with no enclosing damaged callable/block entry of its
/// own, since `build_ir`'s classifier never calls it a callable or block --
/// used to be entirely absent from `entries`, so the sweep's
/// `open_ancestors` stack never opened an excluded ancestor for it. An
/// entity nested directly inside such a span (no callable/block ancestor
/// between them) therefore swept past with `ancestor_excluded == false` and
/// its own `dirty` bit `false` too (nothing walked *into* the damage span
/// to mark it), surviving as measured -- while `redact_targets` (built from
/// `damage` directly, independent of this cascade) still unconditionally
/// wiped the same span's `IrNode` subtree, so `find_ir_subtree` came back
/// empty and `metrics::fallback_ir_body` published a fabricated `cc:1
/// sloc:0` measurement instead of dropping the entity. The fix: every bare
/// damage span now joins `entries` too, as `(span, true, None)`, pushed
/// ahead of the callable/block entries below so a tie at the same
/// `start_byte` still opens the damage entry first (its own `end_byte` is
/// what should win the ancestor race in that case, per the same tie-break
/// this function's own doc comment above already establishes for
/// same-start entity spans). A damage entry always has `own_dirty == true`
/// by construction and carries no entity index (`EntityRef` is `None`), so
/// the sweep's exclusion-write step below only writes into
/// `callable_excluded`/`block_excluded` when the entry's `EntityRef` is
/// `Some` -- a damage entry still opens (and later closes) an ancestor
/// frame on `open_ancestors` exactly like every other entry, it simply has
/// no side table of its own to write `excluded` into. This is a purely
/// additive change to the sweep's entry set and tie-break; the cascade
/// mechanism itself (bottom-up dirty fold, document-order interval
/// nesting) is unchanged from round 3.
fn cascade_exclusions(
    damage: &[DamageSpan],
    callables: &[IrCallable],
    callable_dirty: &[bool],
    blocks: &[IrBlock],
    block_dirty: &[bool],
    owners: &[Span],
) -> Exclusions {
    let mut entries: Vec<(Span, bool, Option<EntityRef>)> =
        Vec::with_capacity(damage.len() + callables.len() + blocks.len() + owners.len());
    for entry in damage {
        entries.push((entry.span, true, None));
    }
    for (index, callable) in callables.iter().enumerate() {
        entries.push((
            callable.span,
            callable_dirty[index],
            Some(EntityRef::Callable(index)),
        ));
    }
    for (index, block) in blocks.iter().enumerate() {
        entries.push((
            block.span,
            block_dirty[index],
            Some(EntityRef::Block(index)),
        ));
    }
    // A7 (WS-4): an owner entry carries no `own_dirty` of its own (`false`
    // unconditionally) -- it is excluded purely via `ancestor_excluded`
    // below, which is why owner entries are pushed last: when an owner
    // shares its callable's exact span (the common case, an owner-kind node
    // that is itself the callable), the callable's own entry above must be
    // swept first so its frame is still open on `open_ancestors` when this
    // owner entry is processed, letting `ancestor_excluded` see the
    // callable's own exclusion rather than a still-default `false`.
    for (index, span) in owners.iter().enumerate() {
        entries.push((*span, false, Some(EntityRef::Owner(index))));
    }
    // Row-1's security fix depends on `sort_by`'s stability: for two
    // entries with an identical span, it preserves push order, and damage
    // entries are pushed first (above), so a damage entry always sorts
    // ahead of a same-span callable/block entry and wins the tie-break
    // below. `sort_unstable_by` would not preserve that push order and
    // could reopen row-1 for that exact same-span shape.
    entries.sort_by(|(a_span, ..), (b_span, ..)| {
        a_span
            .start_byte
            .cmp(&b_span.start_byte)
            .then_with(|| b_span.end_byte.cmp(&a_span.end_byte))
    });

    let mut callable_excluded = vec![false; callables.len()];
    let mut block_excluded = vec![false; blocks.len()];
    let mut owner_excluded = vec![false; owners.len()];
    // One entry per still-open ancestor entity: its own `end_byte` (so the
    // sweep knows when it has moved past it) and whether it is itself
    // excluded.
    let mut open_ancestors: Vec<(u32, bool)> = Vec::new();
    for (span, own_dirty, entity_ref) in entries {
        while let Some(&(end_byte, _)) = open_ancestors.last() {
            if end_byte <= span.start_byte {
                open_ancestors.pop();
            } else {
                break;
            }
        }
        let ancestor_excluded = open_ancestors.last().is_some_and(|&(_, excluded)| excluded);
        let excluded = own_dirty || ancestor_excluded;
        if let Some(entity_ref) = entity_ref {
            match entity_ref {
                EntityRef::Callable(index) => callable_excluded[index] = excluded,
                EntityRef::Block(index) => block_excluded[index] = excluded,
                EntityRef::Owner(index) => owner_excluded[index] = excluded,
            }
        }
        open_ancestors.push((span.end_byte, excluded));
    }

    Exclusions {
        callables: callable_excluded,
        blocks: block_excluded,
        owners: owner_excluded,
    }
}

/// Damage-redacted rebuild of `root` (WS-6 salvage, see `lower_file`'s own
/// doc comment): a node is replaced by a span-only `IrNode::empty` the
/// moment its own span is exactly one of `targets` -- every bare damage
/// span plus every excluded callable's/block's own span (unique to the one
/// tree-sitter node it was built from) -- otherwise every child is rebuilt
/// the same way. Matching by exact span rather than damage-overlap is
/// deliberate: a container node's span always contains its damaged
/// descendant's span too, so an overlap check here would redact the whole
/// file, not just the damaged region. `targets` is a `HashSet` (WS-6 round
/// 3, perf HIGH): O(1) average membership per node, so this whole rebuild
/// stays `Θ(nodes)` regardless of how much damage or how many entities are
/// excluded. An explicit work-list of frames, not recursion, for the same
/// D18 stack-safety reason `build_ir` itself is iterative: a single-child
/// chain thousands of levels deep must not grow the native call stack by
/// one frame per level.
fn prune_damage(root: IrNode, targets: &HashSet<Span>) -> IrNode {
    if targets.is_empty() {
        return root;
    }
    if targets.contains(&root.span) {
        return IrNode::empty(root.span);
    }

    struct Frame {
        span: Span,
        executable: bool,
        decision: Option<DecisionKind>,
        terminator: Option<TerminatorKind>,
        in_block: bool,
        in_catch_body: bool,
        is_comment: bool,
        is_named: bool,
        is_clone_statement: bool,
        is_hoisted_or_type_only: bool,
        pending_children: std::vec::IntoIter<IrNode>,
        rebuilt: Vec<IrNode>,
    }

    fn open_frame(mut node: IrNode) -> Frame {
        // `IrNode` has a custom `Drop` impl (for its own stack-safe
        // teardown), so its fields cannot be moved out by destructuring --
        // every scalar field is `Copy`, read off `&node` first, and
        // `children` is lifted out via `mem::take` before `node` itself
        // (now holding only a dropped-cheap empty `Vec`) goes out of scope.
        let children = std::mem::take(&mut node.children);
        let pending_count = children.len();
        Frame {
            span: node.span,
            executable: node.executable,
            decision: node.decision,
            terminator: node.terminator,
            in_block: node.in_block,
            in_catch_body: node.in_catch_body,
            is_comment: node.is_comment,
            is_named: node.is_named,
            is_clone_statement: node.is_clone_statement,
            is_hoisted_or_type_only: node.is_hoisted_or_type_only,
            pending_children: children.into_iter(),
            rebuilt: Vec::with_capacity(pending_count),
        }
    }

    fn finish(frame: Frame) -> IrNode {
        IrNode {
            span: frame.span,
            executable: frame.executable,
            decision: frame.decision,
            terminator: frame.terminator,
            in_block: frame.in_block,
            in_catch_body: frame.in_catch_body,
            is_comment: frame.is_comment,
            is_named: frame.is_named,
            is_clone_statement: frame.is_clone_statement,
            is_hoisted_or_type_only: frame.is_hoisted_or_type_only,
            children: frame.rebuilt,
        }
    }

    let mut stack = vec![open_frame(root)];
    loop {
        let next_child = stack.last_mut().unwrap().pending_children.next();
        let Some(child) = next_child else {
            let frame = stack.pop().unwrap();
            let finished = finish(frame);
            let Some(parent) = stack.last_mut() else {
                return finished;
            };
            parent.rebuilt.push(finished);
            continue;
        };
        if targets.contains(&child.span) {
            let empty = IrNode::empty(child.span);
            stack.last_mut().unwrap().rebuilt.push(empty);
        } else {
            stack.push(open_frame(child));
        }
    }
}

/// One node's normalized facts, before it is wrapped as an `IrNode`. Kept
/// separate from `IrNode` itself so `src/lower/java.rs` / `jsts.rs` classify
/// a node without needing to know `IrNode`'s `children` field exists.
struct Classification {
    decision: Option<DecisionKind>,
    terminator: Option<TerminatorKind>,
    in_block: bool,
    /// Whether this exact node is the block directly forming a `catch`
    /// clause's body -- an O(1) check per node. `IrNode::in_catch_body`
    /// (also true for every node *inside* that block) is computed by
    /// `build_ir` from this plus the parent's own already-computed flag,
    /// inherited during the traversal rather than re-derived per node by
    /// walking back up to the root (which would turn one linear tree build
    /// into `Θ(depth)` work per node -- `Θ(n²)` on a file whose nesting
    /// depth scales with its size, the pathological case
    /// `test_deeply_nested_file_does_not_abort_the_scan` exists to catch).
    is_catch_body_root: bool,
    damage: Option<DamageKind>,
    /// `exec_lines::is_comment_kind`, computed once here and reused for
    /// `IrNode::is_comment` and (with `is_named` below) `is_clone_statement`'s
    /// own guard, rather than re-derived by both readers.
    is_comment: bool,
    /// Tree-sitter's own `node.is_named()`, computed once here for the same
    /// reason as `is_comment` above.
    is_named: bool,
    /// D15: this node is a direct statement child of one of the six
    /// clone-candidate containers.
    is_clone_statement: bool,
    /// JS/TS only; always `false` from the Java lowering.
    is_hoisted_or_type_only: bool,
    /// This exact node's own canonical block-kind literal (the self-is-block
    /// predicate for `SyntaxBlock` classification), `None` if it is not a
    /// block-kind node. `&'static str`, not `node.kind()` directly: tree-sitter
    /// 0.27's `Node::kind()` borrows from the node's own lifetime rather than
    /// promising `'static`, but `IrBlock::kind` (below) is `&'static str`
    /// (`model::SyntaxBlock::kind` must not move), so each lowering's
    /// `is_block_kind` returns its matched arm's own `'static` literal instead.
    block_kind: Option<&'static str>,
    /// `Some` when this node is a callable-kind node that has a body (D8).
    callable: Option<CallableInfo>,
    /// M1-7: `Some` when this exact node is itself an owner-chain segment --
    /// a named type, an anonymous class body, a namespace, or (reusing
    /// `callable` above) the callable itself. `build_ir` pushes this onto its
    /// own `owner_stack` for the node's descendants once classification
    /// finishes, and pops it again when the node's own frame closes.
    owner_segment: Option<OwnerSegment>,
}

/// D8's per-callable facts a lowering computes once it has already found a
/// callable-kind node with a body: the body node's own span, and D10's
/// resolved name. The declaration node's own span is `build_ir`'s `span`
/// (already computed for every node), so it is not repeated here.
struct CallableInfo {
    body_span: Span,
    name: String,
    /// M1-7: this callable's grammar-free kind.
    kind: CallableKind,
    /// M1-7: whether this callable has no declared name of its own -- see
    /// `IrCallable::is_anonymous`'s own doc comment.
    is_anonymous: bool,
    /// M1-7: the Java parameter-type signature; always empty for JS/TS.
    signature: Vec<String>,
}

/// `parent` is `build_ir`'s already-threaded parent `Node` (see that
/// function's own doc comment for why: `node.parent()` restarts at the tree
/// root in tree-sitter 0.25.10, so threading it down the traversal keeps the
/// whole lowering `Θ(n)` instead of `Θ(n·depth)`).
fn classify(
    node: Node,
    language: LanguageFamily,
    source: &str,
    parent: Option<Node>,
    field_name: Option<&str>,
) -> Classification {
    match language {
        LanguageFamily::Java => java::classify(node, source, parent),
        LanguageFamily::JsTs => jsts::classify(node, source, parent, field_name),
    }
}

/// Whether `node` is a clone-candidate container's direct statement child --
/// the same granularity `clones::statement_children`'s containers enumerate,
/// re-derived here (that function is private to `src/clones/mod.rs`, and
/// this stream may not widen it). `java::is_clone_statement`/
/// `jsts::is_clone_statement` are now production (their result is
/// `IrNode::is_clone_statement`); this dispatcher itself stays test-only --
/// it exists only for the tests below, which need to enumerate the
/// tree-sitter nodes they compare the lowering's tokens against, and derives
/// `node.parent()` and its own field name by scanning directly (rather than
/// threading them, as `build_ir` does for the production path) since nothing
/// here runs against the deep-nesting perf fixture.
#[cfg(test)]
fn is_clone_statement(node: Node, language: LanguageFamily) -> bool {
    let kind = node.kind();
    let parent = node.parent();
    let parent_kind = parent.map(|parent| parent.kind());
    let is_named = node.is_named();
    let is_comment = is_comment_kind(kind, language);
    match language {
        LanguageFamily::Java => java::is_clone_statement(kind, parent_kind, is_named, is_comment),
        LanguageFamily::JsTs => {
            let field_name = parent.and_then(|parent| {
                let mut cursor = parent.walk();
                let index = parent
                    .children(&mut cursor)
                    .position(|child| child.id() == node.id())?;
                parent.field_name_for_child(index as u32)
            });
            jsts::is_clone_statement(parent_kind, field_name, is_named, is_comment)
        }
    }
}

/// D14: one statement node's own normalized token stream -- every leaf
/// descendant in order, comments dropped, an anonymous leaf reduced to its
/// `node.kind()`, a named leaf's text preserved verbatim. Deliberately a
/// fresh implementation of the same algorithm
/// `clones::normalized_statement_tokens` uses, not a call to it: the floor
/// prototype (`nsd-plan-final.md` M0b item 4) exists to prove this
/// independent derivation agrees with the pre-IR one, not to wrap it.
/// Test-only -- see `STATEMENT_TOKEN_SEPARATOR`'s doc comment.
#[cfg(test)]
fn statement_token_stream(statement: Node, language: LanguageFamily, source: &str) -> String {
    let mut tokens = String::new();
    for_each_descendant(statement, |node| {
        if node.child_count() != 0 || is_comment_kind(node.kind(), language) {
            return;
        }
        tokens.push(STATEMENT_TOKEN_SEPARATOR);
        if node.is_named() {
            tokens.push_str(node.utf8_text(source.as_bytes()).unwrap_or(""));
        } else {
            tokens.push_str(node.kind());
        }
    });
    tokens
}

/// A structurally-unreachable fallback `IrNode` for the two dead branches in
/// `build_ir` below: `stack` always holds exactly one frame per node
/// currently open between the traversal's `goto_first_child` into it and
/// its own finalization here, so it cannot be found empty, and a `TreeCursor`
/// created from `root.walk()` cannot be asked to ascend past `root` before
/// `root`'s own frame has already been finalized and returned. Returning a
/// span-only node rather than panicking keeps this scan-pipeline stage
/// consistent with D18: a defect here degrades, it does not crash the run.
fn fallback_node(root: Node) -> IrNode {
    IrNode::empty(Span::from_node(root))
}

/// `build_ir`'s five out-params, bundled into one struct so the function
/// itself stays under clippy's `too_many_arguments` threshold (WS-6 round
/// 3 added the last two fields; five separate `&mut Vec<_>` parameters
/// alongside `root`/`language`/`source` would have pushed it to eight).
struct IrTables<'a> {
    damage: &'a mut Vec<DamageSpan>,
    callables: &'a mut Vec<IrCallable>,
    blocks: &'a mut Vec<IrBlock>,
    /// Index-aligned with `callables`: `callable_dirty[i]` is `callables[i]`'s
    /// own bottom-up "does this subtree touch damage anywhere" bit.
    callable_dirty: &'a mut Vec<bool>,
    /// Index-aligned with `blocks`, the same way `callable_dirty` is with
    /// `callables`.
    block_dirty: &'a mut Vec<bool>,
    /// M1-7 round 2: the per-file owner table `build_ir`'s own `owner_stack`
    /// (below) indexes into, one `OwnerEntry` push per owner-kind node
    /// regardless of how many callables sit underneath it.
    owners: &'a mut Vec<OwnerEntry>,
    /// A7 (WS-4): index-aligned with `owners`, this exact node's own span --
    /// internal only (no `span` field is added to the public `OwnerEntry`),
    /// so `lower_file` can sweep it through `cascade_exclusions` the same
    /// way it already sweeps `callables`/`blocks`.
    owner_spans: &'a mut Vec<Span>,
}

/// Builds `root`'s whole `IrNode` tree in one iterative pass: a single
/// `TreeCursor` walks the raw tree-sitter tree (D18: no per-AST-depth
/// recursion), assembling the `IrNode` tree as it goes rather than only
/// visiting, via an explicit stack of in-progress `IrNode`s mirroring the
/// cursor's own descent depth. A node is finalized (popped and attached to
/// its parent's `children`) the moment the cursor has no more children and
/// no more siblings to explore under it. `metrics::walk_ir_excluding` is a
/// later, separate pass over this already-built `IrNode` tree, using an
/// explicit work-list rather than a `TreeCursor` -- a different shape from
/// this one, not the same one reused.
fn build_ir(root: Node, language: LanguageFamily, source: &str, tables: &mut IrTables) -> IrNode {
    // `parent_in_catch_body` is the already-computed `in_catch_body` flag of
    // this node's own parent (or `false` for `root`), inherited rather than
    // re-derived -- see `Classification::is_catch_body_root`'s doc comment.
    // `parent` is this node's own parent `Node` (`None` for `root`),
    // threaded down the traversal by the caller for the same reason: a
    // `node.parent()` call restarts at the tree root and descends in
    // tree-sitter 0.25.10, so re-deriving it per node would turn this
    // otherwise-linear tree build into `Θ(n·depth)` work.
    //
    // WS-6 round 3: this same closure also seeds each opened node's own
    // "does my own span carry damage" bit (`self_damage`) and, if the node
    // is itself a callable or block, reserves its slot in
    // `callable_dirty_out`/`block_dirty_out` (index-aligned with
    // `callables_out`/`blocks_out`) up front, at `false`, for the main
    // traversal loop's finish-site step to later fold the bottom-up bit
    // into -- see `EntitySlot` and the finish-site comment below.
    // M1-7 round 2 (WS-1 triage row 1, security+perf HIGH): `owner_stack`
    // mirrors `stack`'s own ancestor chain, but holds only the *indices*, into
    // `tables.owners`, of the owner-kind segments among them (named types,
    // anonymous class bodies, namespaces, callables) -- not the segments
    // themselves. A callable's own `owner` is a cheap `Option<u32>` copy of
    // `owner_stack.last()`, taken before this node's own segment (if it has
    // one) is pushed onto it, so no callable carries a deep-copied
    // `Vec<OwnerSegment>` of its own: a shared owner (a giant declared name,
    // or a deeply nested chain) is stored exactly once in `tables.owners`
    // regardless of how many callables it owns. `open` pushes that owner
    // entry itself, once classification names it, and reports whether it did
    // (`owner_pushed`) so the finish-site pop below stays paired one-for-one
    // with the push, exactly mirroring `entity_slots`' own pairing with
    // `callable_dirty`/`block_dirty`. `identity::callable_identity`
    // materializes the full chain on demand by walking `OwnerEntry::parent`.
    let mut owner_stack: Vec<u32> = Vec::new();
    let open = |node: Node,
                parent: Option<Node>,
                parent_in_catch_body: bool,
                field_name: Option<&str>,
                tables: &mut IrTables,
                owner_stack: &mut Vec<u32>|
     -> (IrNode, bool, bool, Option<EntitySlot>, bool) {
        let span = Span::from_node(node);
        let classification = classify(node, language, source, parent, field_name);
        let self_damage = classification.damage.is_some();
        if let Some(kind) = classification.damage {
            tables.damage.push(DamageSpan { kind, span });
        }
        let mut entity_slot = None;
        if let Some(block_kind) = classification.block_kind {
            tables.blocks.push(IrBlock {
                span,
                kind: block_kind,
            });
            tables.block_dirty.push(false);
            entity_slot = Some(EntitySlot::Block(tables.block_dirty.len() - 1));
        }
        if let Some(callable) = classification.callable {
            tables.callables.push(IrCallable {
                span,
                body_span: callable.body_span,
                name: callable.name,
                kind: callable.kind,
                is_anonymous: callable.is_anonymous,
                signature: callable.signature,
                owner: owner_stack.last().copied(),
            });
            tables.callable_dirty.push(false);
            debug_assert!(
                entity_slot.is_none(),
                "node is both block- and callable-shaped"
            );
            entity_slot = Some(EntitySlot::Callable(tables.callable_dirty.len() - 1));
        }
        let owner_pushed = classification.owner_segment.is_some();
        if let Some(segment) = classification.owner_segment {
            tables.owners.push(OwnerEntry {
                segment,
                parent: owner_stack.last().copied(),
            });
            tables.owner_spans.push(span);
            owner_stack.push((tables.owners.len() - 1) as u32);
        }
        let in_catch_body = classification.is_catch_body_root || parent_in_catch_body;
        let ir_node = IrNode {
            span,
            executable: is_executable_leaf(node, language),
            decision: classification.decision,
            terminator: classification.terminator,
            in_block: classification.in_block,
            in_catch_body,
            is_comment: classification.is_comment,
            is_named: classification.is_named,
            is_clone_statement: classification.is_clone_statement,
            is_hoisted_or_type_only: classification.is_hoisted_or_type_only,
            children: Vec::with_capacity(node.child_count() as usize),
        };
        (
            ir_node,
            in_catch_body,
            self_damage,
            entity_slot,
            owner_pushed,
        )
    };

    let mut cursor = root.walk();
    let (root_node, root_in_catch_body, root_self_damage, root_entity_slot, root_owner_pushed) =
        open(root, None, false, None, tables, &mut owner_stack);
    let mut stack: Vec<IrNode> = vec![root_node];
    // Mirrors `stack`'s depth exactly: `catch_flags[i]` is `stack[i]`'s own
    // `in_catch_body` flag, so a child node reads its parent's flag in O(1)
    // via `catch_flags.last()` instead of walking back up the tree.
    let mut catch_flags: Vec<bool> = vec![root_in_catch_body];
    // Mirrors `stack`'s depth exactly too: `parents[i]` is the tree-sitter
    // `Node` that `stack[i]` was itself opened from, so a child node reads
    // its parent's `Node` in O(1) via `parents.last()` instead of via
    // `node.parent()`.
    let mut parents: Vec<Node> = vec![root];
    // WS-6 round 3: `dirty[i]` is `stack[i]`'s own "does this subtree touch
    // damage anywhere so far" bit, seeded from that node's own
    // `self_damage` and OR'd with every child's own folded-in bit at the
    // finish site below, exactly mirroring how `catch_flags` inherits down
    // -- except this one folds bottom-up. `entity_slots[i]` is `Some` iff
    // `stack[i]` is itself a callable or block, naming which of
    // `callable_dirty_out`/`block_dirty_out` its own final bit belongs in.
    let mut dirty: Vec<bool> = vec![root_self_damage];
    let mut entity_slots: Vec<Option<EntitySlot>> = vec![root_entity_slot];
    let mut owner_pushed_frames: Vec<bool> = vec![root_owner_pushed];
    loop {
        if cursor.goto_first_child() {
            let parent_in_catch_body = *catch_flags.last().unwrap_or(&false);
            let parent = *parents.last().unwrap_or(&root);
            let field_name = cursor.field_name();
            let (node, in_catch_body, self_damage, entity_slot, owner_pushed) = open(
                cursor.node(),
                Some(parent),
                parent_in_catch_body,
                field_name,
                tables,
                &mut owner_stack,
            );
            stack.push(node);
            catch_flags.push(in_catch_body);
            parents.push(cursor.node());
            dirty.push(self_damage);
            entity_slots.push(entity_slot);
            owner_pushed_frames.push(owner_pushed);
            continue;
        }
        loop {
            let Some(finished) = stack.pop() else {
                return fallback_node(root);
            };
            catch_flags.pop();
            parents.pop();
            let own_dirty = dirty.pop().unwrap_or(false);
            if let Some(slot) = entity_slots.pop().unwrap_or(None) {
                match slot {
                    EntitySlot::Callable(index) => tables.callable_dirty[index] = own_dirty,
                    EntitySlot::Block(index) => tables.block_dirty[index] = own_dirty,
                }
            }
            if owner_pushed_frames.pop().unwrap_or(false) {
                owner_stack.pop();
            }
            let Some(parent) = stack.last_mut() else {
                return finished;
            };
            parent.children.push(finished);
            if let Some(parent_dirty) = dirty.last_mut() {
                *parent_dirty |= own_dirty;
            }
            if cursor.goto_next_sibling() {
                let parent_in_catch_body = *catch_flags.last().unwrap_or(&false);
                let parent_node = *parents.last().unwrap_or(&root);
                let field_name = cursor.field_name();
                let (node, in_catch_body, self_damage, entity_slot, owner_pushed) = open(
                    cursor.node(),
                    Some(parent_node),
                    parent_in_catch_body,
                    field_name,
                    tables,
                    &mut owner_stack,
                );
                stack.push(node);
                catch_flags.push(in_catch_body);
                parents.push(cursor.node());
                dirty.push(self_damage);
                entity_slots.push(entity_slot);
                owner_pushed_frames.push(owner_pushed);
                break;
            }
            if !cursor.goto_parent() {
                return fallback_node(root);
            }
        }
    }
}

/// Which of `callable_dirty_out`/`block_dirty_out` (see `build_ir`) one
/// stack frame's own bottom-up dirty bit is destined for, if it is a
/// callable/block entity at all -- `None` for every other node.
#[derive(Clone, Copy)]
enum EntitySlot {
    Callable(usize),
    Block(usize),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tree_sitter::Parser;

    use super::*;
    use crate::clones;
    use crate::model::{Grammar, LanguageFamily};

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    /// Every `.java/.js/.jsx/.mjs/.cjs/.ts/.tsx` file under `tests/fixtures/`,
    /// recursively -- a plain directory walk, not `discover::discover`,
    /// since a gitignore/exclude rule would silently narrow "every fixture"
    /// to less than the acceptance requires.
    fn every_fixture_file(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                every_fixture_file(&path, out);
                continue;
            }
            let extension = path.extension().and_then(|extension| extension.to_str());
            if extension
                .is_some_and(|extension| LanguageFamily::from_extension(extension).is_some())
            {
                out.push(path);
            }
        }
    }

    fn grammar_language(path: &Path) -> Option<(Grammar, LanguageFamily)> {
        let extension = path.extension().and_then(|extension| extension.to_str())?;
        let grammar = Grammar::for_extension(extension)?;
        let language = LanguageFamily::from_extension(extension)?;
        Some((grammar, language))
    }

    fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
        match grammar {
            Grammar::Java => tree_sitter_java_orchard::LANGUAGE.into(),
            Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// The IR carries the clone floor (`nsd-plan-final.md` M0b item 4): for
    /// every damage-clear clone-candidate statement in every in-repo
    /// fixture -- including a fixture whose tree carries damage elsewhere,
    /// since WS-6 salvage means `clones::run` now sees exactly those
    /// statements in production too (`parse::parse_all` no longer drops a
    /// whole file for `has_error`; only a damaged entity's own statements
    /// are excluded, via `lower_file`'s salvage pruning) -- the lowering's
    /// independently-derived token stream is byte-identical to
    /// `clones::normalized_statement_tokens`'s. A statement inside damage is
    /// skipped here, not compared: its own tokens are meaningless (an ERROR
    /// node has no stable lexical shape), and `clones::run` never sees it in
    /// production either, since `prune_damage` has already redacted it.
    #[test]
    fn test_ir_tokens_reproduce_the_pre_ir_stream_on_every_fixture() {
        let mut files = Vec::new();
        every_fixture_file(&fixtures_root(), &mut files);
        assert!(!files.is_empty(), "expected at least one fixture file");

        let mut compared = 0usize;
        // WS-6 round 4 (code + security MEDIUM, merged: `compared > 0` alone
        // is satisfiable by the clean fixtures alone, so re-inserting a skip
        // on `tree.root_node().has_error()` after `parser.parse` would leave
        // this test green while silently losing every damaged-tree
        // comparison the round-3 rewrite exists to add). Counts a fixture
        // toward `damaged_files` only once it both has a parse error *and*
        // actually contributed at least one damage-clear comparison, so a
        // reintroduced whole-file skip on `has_error()` -- which would zero
        // every damaged fixture's own `compared_in_file` -- is caught here
        // even though `compared` itself would still be positive from the
        // clean fixtures.
        let mut damaged_files = 0usize;
        for path in files {
            let Some((grammar, language)) = grammar_language(&path) else {
                continue;
            };
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let mut parser = Parser::new();
            if parser.set_language(&tree_sitter_language(grammar)).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(&source, None) else {
                continue;
            };

            let mut damage = Vec::new();
            let mut callables = Vec::new();
            let mut blocks = Vec::new();
            let mut callable_dirty = Vec::new();
            let mut block_dirty = Vec::new();
            let mut owners = Vec::new();
            let mut owner_spans = Vec::new();
            let mut tables = IrTables {
                damage: &mut damage,
                callables: &mut callables,
                blocks: &mut blocks,
                callable_dirty: &mut callable_dirty,
                block_dirty: &mut block_dirty,
                owners: &mut owners,
                owner_spans: &mut owner_spans,
            };
            build_ir(tree.root_node(), language, &source, &mut tables);

            let mut statements = Vec::new();
            for_each_descendant(tree.root_node(), |node| {
                if is_clone_statement(node, language) {
                    statements.push(node);
                }
            });

            let mut compared_in_file = 0usize;
            for statement in statements {
                if damage
                    .iter()
                    .any(|d| Span::from_node(statement).intersects(d.span))
                {
                    continue;
                }
                let ir_tokens = statement_token_stream(statement, language, &source);
                let reference = clones::normalized_statement_tokens(statement, language, &source);
                assert_eq!(
                    ir_tokens,
                    reference,
                    "{}:{} statement token mismatch",
                    path.display(),
                    statement.start_position().row + 1
                );
                compared += 1;
                compared_in_file += 1;
            }
            if tree.root_node().has_error() && compared_in_file > 0 {
                damaged_files += 1;
            }
        }
        assert!(compared > 0, "expected at least one comparable statement");
        assert!(
            damaged_files > 0,
            "expected at least one damaged fixture to contribute a damage-clear comparison"
        );
    }

    /// The D11/SLOC requirement: the IR `executable` flag agrees with
    /// `exec_lines::is_executable_leaf` for every damage-clear node the
    /// lowering produces, across every fixture -- including a fixture whose
    /// tree carries damage elsewhere, since WS-6 salvage means
    /// `metrics::run` now sees exactly those nodes in production too (see
    /// the sibling test's own doc comment for why a damaged node itself is
    /// skipped here rather than compared: `is_executable_leaf` has no
    /// defined answer for an ERROR/MISSING node, and `metrics::run` never
    /// sees one in production either, once `prune_damage` has redacted it).
    #[test]
    fn test_executable_flag_matches_the_d11_predicate() {
        let mut files = Vec::new();
        every_fixture_file(&fixtures_root(), &mut files);

        let mut checked = 0usize;
        // See the sibling test's own `damaged_files` comment: same
        // discrimination gap, same fix, one per fixture actually reached
        // (has a parse error *and* contributed at least one damage-clear
        // node comparison).
        let mut damaged_files = 0usize;
        for path in files {
            let Some((grammar, language)) = grammar_language(&path) else {
                continue;
            };
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let mut parser = Parser::new();
            if parser.set_language(&tree_sitter_language(grammar)).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(&source, None) else {
                continue;
            };

            let mut damage = Vec::new();
            let mut callables = Vec::new();
            let mut blocks = Vec::new();
            let mut callable_dirty = Vec::new();
            let mut block_dirty = Vec::new();
            let mut owners = Vec::new();
            let mut owner_spans = Vec::new();
            let mut tables = IrTables {
                damage: &mut damage,
                callables: &mut callables,
                blocks: &mut blocks,
                callable_dirty: &mut callable_dirty,
                block_dirty: &mut block_dirty,
                owners: &mut owners,
                owner_spans: &mut owner_spans,
            };
            let root = build_ir(tree.root_node(), language, &source, &mut tables);

            let mut ir_nodes = Vec::new();
            collect_ir_nodes(&root, &mut ir_nodes);
            let mut ts_nodes = Vec::new();
            for_each_descendant(tree.root_node(), |node| ts_nodes.push(node));

            assert_eq!(ir_nodes.len(), ts_nodes.len(), "{}", path.display());
            let mut checked_in_file = 0usize;
            for (ir_node, ts_node) in ir_nodes.iter().zip(ts_nodes.iter()) {
                if damage
                    .iter()
                    .any(|d| Span::from_node(*ts_node).intersects(d.span))
                {
                    continue;
                }
                let expected = is_executable_leaf(*ts_node, language);
                assert_eq!(
                    ir_node.executable,
                    expected,
                    "{}:{} executable flag mismatch",
                    path.display(),
                    ts_node.start_position().row + 1
                );
                checked += 1;
                checked_in_file += 1;
            }
            if tree.root_node().has_error() && checked_in_file > 0 {
                damaged_files += 1;
            }
        }
        assert!(checked > 0, "expected at least one node checked");
        assert!(
            damaged_files > 0,
            "expected at least one damaged fixture to contribute a damage-clear comparison"
        );
    }

    /// Pre-order collect of every `IrNode` in the tree the lowering
    /// produced -- test-only, so a small recursive walk over an already
    /// fully-materialized (and fixture-sized) tree is fine; the lowering
    /// itself never recurses (see `build_ir`'s own doc comment).
    fn collect_ir_nodes<'a>(node: &'a IrNode, out: &mut Vec<&'a IrNode>) {
        out.push(node);
        for child in &node.children {
            collect_ir_nodes(child, out);
        }
    }

    /// Every clone-candidate statement's own token stream, in document
    /// order, for one fixture file under `tests/fixtures/`.
    fn clone_statement_tokens(path: &str, language: LanguageFamily) -> Vec<String> {
        let full_path = fixtures_root().join(path);
        let (grammar, _) = grammar_language(&full_path).expect("known fixture extension");
        let source = fs::read_to_string(&full_path).expect("read fixture");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_language(grammar))
            .expect("set_language");
        let tree = parser.parse(&source, None).expect("parse");
        let mut tokens = Vec::new();
        for_each_descendant(tree.root_node(), |node| {
            if is_clone_statement(node, language) {
                tokens.push(statement_token_stream(node, language, &source));
            }
        });
        tokens
    }

    /// Every clone-candidate statement's own document-order start line, for
    /// one fixture file under `tests/fixtures/` -- test-only support for
    /// `test_clone_candidate_containers_cover_every_container_arm` below.
    fn clone_statement_start_lines(path: &str, language: LanguageFamily) -> Vec<u32> {
        let full_path = fixtures_root().join(path);
        let (grammar, _) = grammar_language(&full_path).expect("known fixture extension");
        let source = fs::read_to_string(&full_path).expect("read fixture");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_language(grammar))
            .expect("set_language");
        let tree = parser.parse(&source, None).expect("parse");
        let mut lines = Vec::new();
        for_each_descendant(tree.root_node(), |node| {
            if is_clone_statement(node, language) {
                lines.push(node.start_position().row as u32 + 1);
            }
        });
        lines
    }

    /// HIGH-2 (round 2): the clone-floor container set was unproven -- the
    /// suite stayed green with `"constructor_body"`/
    /// `"switch_block_statement_group"` (Java) or `"program"`/
    /// `switch_case`/`switch_default` body field (JS/TS) deleted from
    /// `is_clone_statement`. `ContainerSet.java` exercises a constructor
    /// whose `constructor_body` holds exactly two statements and a `switch`
    /// whose `switch_block_statement_group` holds exactly two non-label
    /// statements (plus the `switch_statement` itself, a `block`-parented
    /// clone candidate already covered elsewhere); `container_set.ts`
    /// exercises two top-level `program` statements plus a top-level
    /// function declaration, and a `switch_case` whose `body` field holds
    /// two statements. Asserts document-order start lines, not just a count,
    /// so a swapped container arm (matching the wrong parent kind) cannot
    /// hide behind a coincidentally-equal total.
    #[test]
    fn test_clone_candidate_containers_cover_every_container_arm() {
        let java_lines = clone_statement_start_lines("ir/ContainerSet.java", LanguageFamily::Java);
        assert_eq!(java_lines, vec![3, 4, 8, 10, 11], "{java_lines:?}");

        let ts_lines = clone_statement_start_lines("ir/container_set.ts", LanguageFamily::JsTs);
        assert_eq!(ts_lines, vec![1, 2, 4, 5, 7, 8], "{ts_lines:?}");
    }

    /// The clone-token floor's equality is discriminating, not vacuous
    /// (`nsd-plan-final.md` M0b item 4): two fixtures whose two-statement
    /// clone body agrees on the first statement and differs on the second
    /// produce token streams that agree at index zero and differ at index
    /// one. Calls `statement_token_stream` directly rather than reading
    /// `IrNode::token` -- that field carried no reader in `src/` and was
    /// removed; WS-4 derives a stream on demand the same way this test does.
    #[test]
    fn test_ir_tokens_differ_when_a_statement_differs() {
        let a_tokens = clone_statement_tokens("ir/DifferA.java", LanguageFamily::Java);
        let b_tokens = clone_statement_tokens("ir/DifferB.java", LanguageFamily::Java);

        assert_eq!(a_tokens.len(), 2, "{a_tokens:?}");
        assert_eq!(b_tokens.len(), 2, "{b_tokens:?}");
        assert_eq!(a_tokens[0], b_tokens[0]);
        assert_ne!(a_tokens[1], b_tokens[1]);
    }

    /// WS-6 round 3 (security MEDIUM: bare/stray damage escaping pruning
    /// entirely, finding (c)): a damage span that sits outside every
    /// callable and block -- `build_ir`'s classifier never calls it a
    /// callable or block, so the entity-level exclusion path in
    /// `lower_file` never sees it as an entity to exclude -- must still be
    /// redacted from the tree via `redact_targets`' bare-damage union
    /// (every `damage_out` span, not just excluded entities' own spans), so
    /// no analyzer that walks `ir_file.root` unconditionally (metrics' own
    /// file-level `scanned_lines`, `clones::run`, `rules::run`) can ever
    /// reach a node whose own span carries damage. The surrounding clean
    /// callable is unaffected: it survives in `ir_file.callables`, and none
    /// of its own nodes intersects a damage span.
    #[test]
    fn test_stray_damage_outside_any_callable_or_block_is_excluded_from_every_analyzer() {
        let source = "export function safe(x) {\n  return x;\n}\n\n)));\n".to_string();
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .expect("set_language");
        let tree = parser.parse(&source, None).expect("parse");
        let parsed = ParsedFile {
            relative_path: PathBuf::from("stray.ts"),
            language: LanguageFamily::JsTs,
            source,
            tree,
        };

        let ir_file = lower_file(&parsed);
        assert!(
            !ir_file.damage.is_empty(),
            "expected at least one damage span from the trailing `)));`"
        );
        assert!(
            ir_file
                .callables
                .iter()
                .any(|callable| callable.name.contains("safe")),
            "expected the clean `safe` callable to survive: {:?}",
            ir_file.callables
        );

        let mut nodes = Vec::new();
        collect_ir_nodes(&ir_file.root, &mut nodes);
        for node in nodes {
            if node.executable || node.is_clone_statement {
                assert!(
                    !ir_file
                        .damage
                        .iter()
                        .any(|entry| node.span.intersects(entry.span)),
                    "expected every executable/clone-candidate node to be damage-clear: {:?}",
                    node.span
                );
            }
        }
    }
}

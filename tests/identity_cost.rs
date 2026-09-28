//! WS-3 A5's own cost bound (task.md A5: "The plan should require an
//! end-to-end cost bound or a compact identity representation before
//! marking the hot-path work complete"): `identity::identities`'s total
//! allocation on a deeply nested owner chain must be O(owners), not the
//! O(owners^2) a per-callable `Vec<OwnerSegment>` deep copy used to cost.
//!
//! A counting `#[global_allocator]`, scoped to this file's own test binary
//! (each `tests/*.rs` file compiles as its own separate binary, so this
//! never touches any other test's allocator) -- measures bytes allocated
//! strictly inside the `identities()` call, on a 4,000-deep `()=>` chain.
//! This file holds two tests sharing that one counter; both take
//! `MEASURE_LOCK` as their first statement, so the counter's before/after
//! delta counts only the locked test's own allocations even when `cargo
//! test`'s default runner schedules them on separate threads.
//! Before A5, materializing every callable's own full owner chain (one
//! `Vec<OwnerSegment>` per callable, summed over a 4,000-deep chain) copies
//! on the order of 8,000,000 segments, several times over the 16 MiB bound
//! below; after A5, `identities()` computes one `OwnerDigest` per owner
//! entry (O(owners)) and looks each callable's own digest up in O(1).
//!
//! Source is inline, never a file under `tests/fixtures/`, for the same
//! reason `tests/identity.rs`'s own doc comment gives: growing that corpus
//! would move `tests/golden/neutrality/clean.report.json`'s committed
//! baseline, which this stream's scope forbids touching.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use nsd::identity;
use nsd::lower;
use nsd::model::{Grammar, LanguageFamily};
use nsd::parse::ParsedFile;

/// Total bytes ever handed out by `CountingAllocator::alloc` (and its
/// default-implemented callers `alloc_zeroed`/`realloc`), process-wide.
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Serializes this file's two tests around the one process-wide
/// `ALLOCATED_BYTES` counter, so `cargo test`'s default multi-threaded
/// runner cannot interleave a sibling test's allocations into a locked
/// test's own before/after delta. Poison-tolerant: a prior test panicking
/// while holding the lock must not fail every later test in the same run
/// with a poisoned-mutex panic instead of its own assertion.
static MEASURE_LOCK: Mutex<()> = Mutex::new(());

/// Wraps the real `System` allocator, tallying every allocation's own size
/// into `ALLOCATED_BYTES` -- each of this file's two tests reads the counter
/// before and after its own measured call while holding `MEASURE_LOCK`, so
/// the delta is exactly what that one call allocated, never a sibling test's
/// concurrent allocations.
struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// The same nesting depth `tests/identity.rs::
/// test_deeply_nested_callables_store_owners_linearly` already exercises for
/// the owner *table*'s own linearity; this file measures `identities()`'s
/// own allocation on top of that table.
const DEPTH: usize = 4_000;

/// A5's own bound (task.md A5 / this stream's brief): `identities()`'s total
/// allocation on `DEPTH` nested owners must stay under 16 MiB. About
/// 8,000,000 copied `OwnerSegment`s (well over this bound) is what the
/// pre-A5 per-callable full-chain materialization cost on this same fixture.
const ALLOCATION_BOUND_BYTES: usize = 16 * 1024 * 1024;

/// Mirrors `tests/identity.rs::parse_inline` -- duplicated rather than
/// imported, since each `tests/*.rs` file is its own compiled binary (see
/// that file's own doc comment).
fn parse_inline(source: &str, grammar: Grammar, language: LanguageFamily) -> ParsedFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language(grammar))
        .expect("set_language");
    let tree = parser.parse(source, None).expect("parse");
    ParsedFile {
        relative_path: PathBuf::from("inline"),
        language,
        source: source.to_string(),
        tree,
    }
}

fn tree_sitter_language(grammar: Grammar) -> tree_sitter::Language {
    match grammar {
        Grammar::Java => tree_sitter_java_orchard::LANGUAGE.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

/// The observable acceptance test: fails before A5 (identities() on this
/// fixture allocates well over 16 MiB), passes after.
#[test]
fn test_identities_allocation_is_linear_in_owners_on_a_deep_chain() {
    let _serial = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!("const f = {}0;", "()=>".repeat(DEPTH));
    let parsed = parse_inline(&source, Grammar::TypeScript, LanguageFamily::JsTs);
    let ir_file = lower::lower_file(&parsed);
    assert_eq!(
        ir_file.callables.len(),
        DEPTH,
        "fixture must have one callable per nesting level"
    );
    assert_eq!(
        ir_file.owners.len(),
        DEPTH,
        "fixture must have one owner per nesting level"
    );

    let before = ALLOCATED_BYTES.load(Ordering::SeqCst);
    let identities = identity::identities(&ir_file);
    let after = ALLOCATED_BYTES.load(Ordering::SeqCst);

    assert_eq!(identities.len(), DEPTH);
    let allocated = after - before;
    assert!(
        allocated < ALLOCATION_BOUND_BYTES,
        "identities() allocated {allocated} bytes building {DEPTH} identities, expected under {ALLOCATION_BOUND_BYTES} (16 MiB)"
    );
}

/// Round 2 triage row 3: a single `callable_identity` call on the fixture's
/// own outermost callable (document order, no lexical owner -- see
/// `tests/identity.rs`'s own `owner_chain` doc comment for why a top-level
/// callable's `owner` is `None`) must allocate far less than `DEPTH *
/// size_of::<OwnerDigest>()` bytes. This is the mutant this bound actually
/// discriminates: a `callable_identity` that (re)builds the whole file's
/// `owner_digests` table before looking at `callable.owner` at all would
/// allocate O(owners) regardless of that callable's own owner, even though
/// this one owner is `None` and its own O(depth) walk should touch nothing.
#[test]
fn test_single_callable_identity_call_does_not_build_the_whole_table() {
    let _serial = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!("const f = {}0;", "()=>".repeat(DEPTH));
    let parsed = parse_inline(&source, Grammar::TypeScript, LanguageFamily::JsTs);
    let ir_file = lower::lower_file(&parsed);
    let outermost = &ir_file.callables[0];
    assert_eq!(
        outermost.owner, None,
        "fixture's own document-order first callable must be the top-level, owner-less one"
    );

    let single_call_bound_bytes = DEPTH * std::mem::size_of::<identity::OwnerDigest>();

    let before = ALLOCATED_BYTES.load(Ordering::SeqCst);
    let single_identity = identity::callable_identity(&ir_file, outermost);
    let after = ALLOCATED_BYTES.load(Ordering::SeqCst);

    assert_eq!(
        single_identity.owner_digest,
        identity::OwnerDigest::default()
    );
    let allocated = after - before;
    assert!(
        allocated < single_call_bound_bytes,
        "a single callable_identity() call allocated {allocated} bytes, expected under {single_call_bound_bytes} (DEPTH * size_of::<OwnerDigest>())"
    );
}

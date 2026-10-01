//! M0b: the normalized IR (`nsd-plan-final.md`, *Architecture* -> *The D-IR
//! pipeline*). A single node type shared by both grammars, carrying exactly
//! what the *What the IR must carry* table names: a byte-and-line `Span`
//! back to the original source, an `executable` flag (D11), a uniform
//! `DecisionKind` (no per-grammar strings), typed damage spans, and the
//! three structural predicates the six rules need (terminator-ness, block
//! membership, catch-body membership). `src/lower/` is the only producer of
//! `IrNode` values; this module carries no grammar-string matching itself
//! (`node.kind()` literals live in `src/lower/java.rs` / `src/lower/jsts.rs`
//! only) so a lowering-isolation check can hold it to that.

use tree_sitter::Node;

/// M0c's fingerprint keys on this alongside the two lowering versions below;
/// bump it whenever `IrNode`'s shape changes in a way that would change what
/// a downstream consumer reads off it. Bumped 2 -> 3 for M0c-9: removing
/// `DamageKind::JavaVarargsAnnotation` changes what a `DamageSpan` can carry.
/// Bumped 3 -> 4 for M1-7 (line-independent callable identity): `IrCallable`
/// gains `kind`, `is_anonymous`, `signature` and `owner` (round 2: an index
/// into `lower::IrFile::owners`, not a deep-copied chain), changing what a
/// downstream consumer -- WS-1's own `identity` module -- reads off it. The
/// round-2 owner-storage change is a memory-safety fix to the same
/// `IrCallable` shape this bump already covers, not a further shape bump of
/// its own. Bumped 4 -> 5 for M3-4 (A101): `lower::IrFile` gains
/// `excluded_callables`, the spans of the callables salvage dropped, which the
/// parse-damage policy reads.
pub const IR_VERSION: u32 = 5;

/// A byte-and-line span back into the original source text a `ParsedFile`
/// holds (D11/SLOC's requirement on the IR): both a byte range, for exact
/// text extraction, and a 1-based line range, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub end_line: u32,
}

impl Span {
    /// Reads a tree-sitter node's own span directly; the one place `Span`
    /// touches a tree-sitter type, since that is exactly what it is meant to
    /// mirror, not a grammar-string match. `as u32` is lossless here: the
    /// underlying tree-sitter C fields these wrap are themselves `uint32_t`.
    pub fn from_node(node: Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            start_line: node.start_position().row as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
        }
    }

    /// Whether `self` and `other` share at least one byte -- the salvage
    /// query's primitive: a callable "intersects damage" the moment its own
    /// span overlaps a damage span at all, not only when one fully contains
    /// the other, so a partially-damaged declaration still counts.
    ///
    /// WS-6 round 3 (security MEDIUM: boundary damage): a zero-width span
    /// (`start_byte == end_byte`, tree-sitter's usual shape for an inserted
    /// `MISSING` node -- a truncated file's `MISSING "}"` sits exactly at
    /// its enclosing callable's own `end_byte`) can never satisfy the
    /// strict `<` test below on its own side, so it is checked first and
    /// inclusively on both boundaries: a zero-width `other` intersects
    /// `self` iff `self` contains that one byte position,
    /// `self.start_byte <= other.start_byte <= self.end_byte` (and
    /// symmetrically for a zero-width `self`). Two non-empty spans keep the
    /// original strict two-sided test unchanged: merely touching at one
    /// boundary byte is not an overlap for either.
    pub fn intersects(self, other: Span) -> bool {
        if other.start_byte == other.end_byte {
            return self.start_byte <= other.start_byte && other.start_byte <= self.end_byte;
        }
        if self.start_byte == self.end_byte {
            return other.start_byte <= self.start_byte && self.start_byte <= other.end_byte;
        }
        self.start_byte < other.end_byte && other.start_byte < self.end_byte
    }
}

/// The `cc` consumer's uniform decision-node kind (*What the IR must carry*):
/// one enum shared by both lowerings rather than two per-grammar string
/// tables. `&&`/`||` are `And`/`Or`; `if` is `Branch`; every loop form
/// (`for`/enhanced-`for`/`for-in`/`for-of`/`while`/`do`) is `Loop`; a
/// non-default `switch` label is `Case`; `catch` is `Catch`; `a ? b : c` is
/// `Ternary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Branch,
    Loop,
    Case,
    Catch,
    Ternary,
    And,
    Or,
}

/// The two known damage classes from `nsd-plan-final.md`'s *The 16 parse
/// failures* table that survive under orchard, plus a catch-all for an
/// `ERROR`/`MISSING` node the lowering does not recognize as one of them.
/// Typed, not just "there was an error somewhere" (*Architecture* -> salvage
/// row). `JavaVarargsAnnotation` (M0c-9/M0c-10) is removed here: orchard's
/// grammar no longer produces the `ERROR` shape it named, so every Java
/// `ERROR`/`MISSING` node now falls back to `Unclassified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageKind {
    TsUsingParameterName,
    JsxUnterminatedEntity,
    Unclassified,
}

/// One typed damage span: what kind of damage, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageSpan {
    pub kind: DamageKind,
    pub span: Span,
}

/// The six rules' terminator-ness classification, narrowed from a single
/// bool to the four kinds `nsd-plan-final.md`'s D22/D7 table names, so the
/// two narrower subsets below (`is_unreachable_terminator`,
/// `is_return_or_throw`) are expressible without a grammar string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminatorKind {
    Return,
    Throw,
    Break,
    Continue,
}

/// M1-7's grammar-free callable classification: one variant per callable-kind
/// grammar node `src/lower/java.rs`/`src/lower/jsts.rs` recognize (each
/// file's own `callable_kind_for`), so `src/identity/`'s `CallableIdentity`
/// never needs a grammar string of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallableKind {
    JavaMethod,
    JavaConstructor,
    JavaCompactConstructor,
    JavaStaticInitializer,
    JavaLambda,
    JsFunctionDeclaration,
    JsGeneratorFunctionDeclaration,
    JsFunctionExpression,
    JsArrowFunction,
    JsMethodDefinition,
}

/// M1-7's lexical owner-chain segment kind: one enclosing named type,
/// anonymous class body, namespace or callable, in nesting order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnerKind {
    NamedType,
    AnonymousClassBody,
    Namespace,
    Callable,
}

/// One segment of a callable's lexical owner chain (M1-7): its kind, and its
/// declared name -- `None` for an anonymous class body, or for a `Callable`
/// segment whose own enclosing callable is itself undeclared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnerSegment {
    pub kind: OwnerKind,
    pub name: Option<String>,
}

/// M1-7 round 2 (security+perf HIGH, WS-1 triage row 1): one entry in
/// `lower::IrFile::owners`, the per-file owner table every owner-kind node
/// (named type, anonymous class body, namespace, callable) contributes
/// exactly one of, regardless of how many callables sit underneath it.
/// `parent` is the index of the next segment out (`None` for an outermost
/// owner), so a callable's whole `owner_chain` is reconstructed by walking
/// `parent` links rather than each callable carrying its own deep-copied
/// `Vec<OwnerSegment>` -- the deep copy is what let a hostile 15,000-level
/// `()=>` nesting or a single giant declared name multiply across every
/// sibling callable into gigabytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEntry {
    pub segment: OwnerSegment,
    pub parent: Option<u32>,
}

/// D8's callable table: one entry per callable-kind node that has a body,
/// in document order, alongside `IrNode`'s per-node tree so per-node memory
/// does not grow to carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrCallable {
    /// The callable declaration node's own span.
    pub span: Span,
    /// The callable's body node's span (the `body` field child, else -- for
    /// Java's `static_initializer`, the one callable kind with no `body`
    /// field -- its first `block` child).
    pub body_span: Span,
    /// D10's resolved name -- the *display* name: `<anonymous>@<line>` when
    /// undeclared. M1-7's `src/identity/` deliberately does not reuse this
    /// as identity (see `is_anonymous` below): its anonymous fallback embeds
    /// the declaration's own line number.
    pub name: String,
    /// M1-7: the grammar-free callable kind.
    pub kind: CallableKind,
    /// M1-7: whether this callable has no declared name of its own (no
    /// `name` field, and no name reachable via the same enclosing
    /// `variable_declarator`/`pair`/`assignment_expression` fallback `name`
    /// above already uses) -- `name`'s own anonymous fallback embeds a line
    /// number and so cannot itself answer this question.
    pub is_anonymous: bool,
    /// M1-7: the Java parameter-type texts, whitespace-normalized, in
    /// declaration order; always empty for JS/TS (untyped JS has no
    /// parameter types, and a TS overload signature without a body is not a
    /// callable).
    pub signature: Vec<String>,
    /// M1-7 round 2 (was `owner_chain: Vec<OwnerSegment>`, WS-1 triage row 1):
    /// the index, into `lower::IrFile::owners`, of this callable's innermost
    /// enclosing owner -- `None` for a top-level callable with no lexical
    /// owner. `identity::callable_identity` walks `OwnerEntry::parent` from
    /// here to materialize the full chain on demand, instead of every
    /// callable carrying its own deep copy of it.
    pub owner: Option<u32>,
}

/// The self-is-block predicate's table: one entry per block-kind node
/// (`SyntaxBlock` classification), in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrBlock {
    pub span: Span,
    /// The grammar `kind` string this block came from -- carried because
    /// `model::SyntaxBlock::kind` is itself a `&'static str` whose value must
    /// not move; produced only inside `src/lower/`, the one directory the
    /// lowering-isolation freeze exempts.
    pub kind: &'static str,
}

/// The normalized node every lowering produces: one node per original
/// tree-sitter node, in the same shape, carrying only the normalized facts
/// the *What the IR must carry* table names.
#[derive(Debug, Clone, PartialEq)]
pub struct IrNode {
    pub span: Span,
    /// D11: this leaf counts toward SLOC (`exec_lines::is_executable_leaf`,
    /// which the lowering calls rather than restating).
    pub executable: bool,
    /// `cc`'s uniform decision-node kind, when this node is one.
    pub decision: Option<DecisionKind>,
    /// The six rules' terminator-ness classification: `return`/`break`/
    /// `continue`/`throw`, identical node-kind literals in both grammars.
    /// `Some` at all is `is_terminator`; the two narrower subsets below key
    /// on which variant.
    pub terminator: Option<TerminatorKind>,
    /// The six rules' block-membership predicate: this node's own immediate
    /// parent is a `{ … }` block (not, e.g., an unbraced `if`'s single
    /// statement body).
    pub in_block: bool,
    /// The six rules' catch-body predicate: this node is the block
    /// directly forming a `catch` clause's body, or sits inside it.
    pub in_catch_body: bool,
    /// `exec_lines::is_comment_kind`, reused rather than restated: the
    /// comment-vs-anonymous-leaf marker (with `is_named` below).
    pub is_comment: bool,
    /// Tree-sitter's own `node.is_named()`: with `is_comment`, separates a
    /// comment from an anonymous punctuation leaf -- both otherwise read as
    /// `executable == false`. Also what `statement_children` ("named,
    /// non-comment direct children") becomes over `IrNode::children`, which
    /// holds anonymous children too.
    pub is_named: bool,
    /// D15: this node is a direct statement child of one of the six
    /// clone-candidate containers (`java::is_clone_statement` /
    /// `jsts::is_clone_statement`).
    pub is_clone_statement: bool,
    /// JS/TS only, always `false` from the Java lowering: a hoisted
    /// function declaration or a type-only declaration, exempt from the
    /// unreachable-after-return rule even though it sits after an
    /// unconditional terminator.
    pub is_hoisted_or_type_only: bool,
    pub children: Vec<IrNode>,
}

impl IrNode {
    /// A structurally-empty `IrNode`. Its main caller today is
    /// `src/lower/`'s own `prune_damage`, redacting a damaged subtree; its
    /// only caller outside `src/lower/` is `metrics::fallback_ir_body`,
    /// D18's degrade-don't-panic fallback there: all flags false, no
    /// decision, no terminator, no children.
    pub fn empty(span: Span) -> IrNode {
        IrNode {
            span,
            executable: false,
            decision: None,
            terminator: None,
            in_block: false,
            in_catch_body: false,
            is_comment: false,
            is_named: false,
            is_clone_statement: false,
            is_hoisted_or_type_only: false,
            children: Vec::new(),
        }
    }
}

impl Drop for IrNode {
    /// A tree this deep (the metrics suite's 15,000-level nesting regression
    /// fixture) overflows the stack under the compiler's default recursive
    /// drop glue: a `Vec<IrNode>` drops each element in turn, and a long
    /// single-child chain turns that into one recursive `drop` call per
    /// level. Converts teardown into the same iterative, stack-safe shape
    /// `build_ir` uses to construct the tree: pop a node, move its own
    /// children onto the pending work-list (leaving its `children` field
    /// empty so its *own* drop, once popped, has nothing left to recurse
    /// into), repeat.
    fn drop(&mut self) {
        let mut pending: Vec<IrNode> = std::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
    }
}

/// The six rules' terminator-ness query: no grammar string, just the flag
/// the lowering already computed.
pub fn is_terminator(node: &IrNode) -> bool {
    node.terminator.is_some()
}

/// The narrower terminator subset `rules::UNREACHABLE_TERMINATOR_KINDS`
/// names: `return`/`throw`/`break`, not `continue`.
pub fn is_unreachable_terminator(node: &IrNode) -> bool {
    matches!(
        node.terminator,
        Some(TerminatorKind::Return) | Some(TerminatorKind::Throw) | Some(TerminatorKind::Break)
    )
}

/// The narrower terminator subset `rules::always_returns` names:
/// `return`/`throw` only.
pub fn is_return_or_throw(node: &IrNode) -> bool {
    matches!(
        node.terminator,
        Some(TerminatorKind::Return) | Some(TerminatorKind::Throw)
    )
}

/// The six rules' block-membership query.
pub fn is_block_member(node: &IrNode) -> bool {
    node.in_block
}

/// The six rules' catch-body query.
pub fn is_in_catch_body(node: &IrNode) -> bool {
    node.in_catch_body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start_byte: u32, end_byte: u32) -> Span {
        Span {
            start_byte,
            end_byte,
            start_line: 1,
            end_line: 1,
        }
    }

    #[test]
    fn test_two_overlapping_non_empty_spans_intersect() {
        assert!(span(0, 10).intersects(span(5, 15)));
        assert!(span(5, 15).intersects(span(0, 10)));
    }

    #[test]
    fn test_two_disjoint_non_empty_spans_do_not_intersect() {
        assert!(
            !span(0, 5).intersects(span(5, 10)),
            "touching at one boundary byte is not overlap for two non-empty spans"
        );
        assert!(!span(0, 5).intersects(span(10, 15)));
    }

    /// The regression repro (WS-6 round 3, security MEDIUM): a truncated
    /// file's `MISSING "}"` is a zero-width span sitting exactly at its
    /// enclosing callable's own `end_byte`.
    #[test]
    fn test_a_zero_width_span_at_the_others_end_byte_intersects() {
        assert!(span(0, 10).intersects(span(10, 10)));
        assert!(span(10, 10).intersects(span(0, 10)));
    }

    #[test]
    fn test_a_zero_width_span_at_the_others_start_byte_intersects() {
        assert!(span(10, 20).intersects(span(10, 10)));
        assert!(span(10, 10).intersects(span(10, 20)));
    }

    #[test]
    fn test_a_zero_width_span_strictly_outside_does_not_intersect() {
        assert!(!span(0, 10).intersects(span(20, 20)));
        assert!(!span(20, 20).intersects(span(0, 10)));
    }

    #[test]
    fn test_two_zero_width_spans_at_the_same_position_intersect() {
        assert!(span(5, 5).intersects(span(5, 5)));
    }

    #[test]
    fn test_two_zero_width_spans_at_different_positions_do_not_intersect() {
        assert!(!span(5, 5).intersects(span(6, 6)));
    }
}

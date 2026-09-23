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
/// a downstream consumer reads off it.
pub const IR_VERSION: u32 = 1;

/// A byte-and-line span back into the original source text a `ParsedFile`
/// holds (D11/SLOC's requirement on the IR): both a byte range, for exact
/// text extraction, and a 1-based line range, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The three known damage classes from `nsd-plan-final.md`'s *The 16 parse
/// failures* table, plus a catch-all for an `ERROR`/`MISSING` node the
/// lowering does not recognize as one of the three. Typed, not just "there
/// was an error somewhere" (*Architecture* -> salvage row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageKind {
    JavaVarargsAnnotation,
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
    /// The six rules' terminator-ness predicate: `return`/`break`/
    /// `continue`/`throw`, identical node-kind literals in both grammars.
    pub is_terminator: bool,
    /// The six rules' block-membership predicate: this node's own immediate
    /// parent is a `{ … }` block (not, e.g., an unbraced `if`'s single
    /// statement body).
    pub in_block: bool,
    /// The six rules' catch-body predicate: this node is the block
    /// directly forming a `catch` clause's body, or sits inside it.
    pub in_catch_body: bool,
    /// The clones consumer's statement-token stream (D14), present only on
    /// a node the lowering identifies as a clone-candidate container's
    /// direct statement child -- the same granularity
    /// `clones::normalized_statement_tokens` computes over.
    pub token: Option<String>,
    pub children: Vec<IrNode>,
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
    node.is_terminator
}

/// The six rules' block-membership query.
pub fn is_block_member(node: &IrNode) -> bool {
    node.in_block
}

/// The six rules' catch-body query.
pub fn is_in_catch_body(node: &IrNode) -> bool {
    node.in_catch_body
}

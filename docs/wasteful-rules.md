# Wasteful-code rules and the adapted verbosity score

**Adaptation, not reimplementation.** scb-check currently documents its
wasteful-code rules as Python-only. The six rules below are an explicit
adaptation of that idea to Java and JS/TS, run over this scanner's own IR
(`src/ir/mod.rs`, `src/rules/mod.rs`) rather than either grammar's raw AST —
chosen and implemented independently, not a port of scb-check's Python
implementation, and this document makes no claim of numerical equivalence
to any score scb-check itself publishes.

## The six v1 rules (D22)

Six rules, closed for v1: three Java, three JS/TS, one pair per shape. Each
is decidable purely from the IR — no control-flow graph, no name
resolution, no cross-file information — which is what "conservative" means
throughout this document: a rule only ever flags a shape that is
unambiguous in isolation, so it can under-report real waste but is designed
to never mislabel legitimate code.

### `JAVA-UNREACHABLE-AFTER-RETURN` / `JSTS-UNREACHABLE-AFTER-RETURN`

Flags every statement that follows an unconditional terminator
(`ir::is_unreachable_terminator`: `IrNode::terminator` is `Some(Return)`,
`Some(Throw)` or `Some(Break)` — not `Continue`) that is itself a direct
statement child of a block-kind container (`IrNode::in_block`; the
containers this covers are named per language under Lowering below) —
except, in JS/TS, a statement `IrNode::is_hoisted_or_type_only` exempts. A
terminator nested inside a conditional does not count: this rule proves
nothing about reachability in general, it only flags the syntactically
obvious case where a block's own trailing statements can never run. That
means it can miss real dead code; it never flags a statement that might
still be reachable.

In JS/TS, `is_hoisted_or_type_only` is set for a hoisted function
declaration (runs regardless of source position) or a type-only declaration
(erased at runtime) — neither is dead code in effect, only in source
position. The exact node kinds this covers are named under Lowering below.

### `JAVA-EMPTY-CATCH` / `JSTS-EMPTY-CATCH`

Flags a `catch` clause (`IrNode::decision == Some(DecisionKind::Catch)`)
whose body — the block directly forming the clause's body, the root of its
`IrNode::in_catch_body` span — has no named child at all: neither a
statement nor a comment. A caught exception whose block still holds an
explanatory comment (e.g. `// intentionally ignored`) is left alone — a
comment is itself a named node, so it is counted the same way a statement
would be, and its presence is evidence the emptiness is a deliberate
choice, not code simply swallowing an error with no trace.

### `JAVA-REDUNDANT-ELSE-AFTER-RETURN` / `JSTS-REDUNDANT-ELSE-AFTER-RETURN`

Flags an `if`'s (`DecisionKind::Branch`) `else` branch when the `if`'s own
consequence branch always returns: the consequence is itself a bare
`return`/`throw` (`ir::is_return_or_throw`), or a block-kind node whose
*last* direct statement is one. No exhaustiveness or dataflow analysis is
attempted: an `if`/`else` chain that returns on every path but does not end
its `if` branch in a literal `return`/`throw` is not flagged. This rule can
miss real redundancy; it never claims a branch returns when it cannot prove
that from syntax alone.

## The verbosity score (D23)

```
verbosity = |{ (file, line) flagged by a rule finding, or by a clone
               group's occurrence past the first }|
            -----------------------------------------------------------
                       scanned source lines (D12)
```

- A rule finding contributes every D11-counted line of the statements it
  actually flags — for `*-UNREACHABLE-AFTER-RETURN` in JS/TS, that excludes
  an exempt declaration (`is_hoisted_or_type_only`, see above) even when it
  sits inside the finding's reported `start_line`–`end_line` span. A
  closing-brace-only (or keyword-only, e.g. a bare `} else {`) line
  contributes nothing, so a flagged construct's own punctuation never
  inflates the score.
- A clone group contributes only the D11-counted lines of every occurrence
  but the canonically first one (`redundant_occurrences`, D14, see
  `docs/clone-detection.md`) — the occurrence a reader would actually keep
  is never itself counted as waste.
- The two contributions are unioned as distinct `(file, line)` positions
  before dividing, so a line flagged by both a rule and a clone counts
  once, not twice.
- The score is reported once overall, and once per language family (D20):
  a Java finding never moves the `js_ts` score, and vice versa.
- The denominator is D12's scanned-line count — every D11-counted line
  across a whole parsed file, not just Σ callable SLOC (D12 explicitly
  covers imports, field and type declarations too, which sit outside every
  callable).
- `0.0` when there are no scanned lines at all (an empty scan or a scan
  where nothing parsed), never a division by zero.
- A file that failed to parse (D18) contributes to neither the numerator
  nor the denominator; the score's `incomplete` marker records that at
  least one file is missing from the picture rather than silently treating
  it as zero waste.

## Lowering: Java (`tree-sitter-java-orchard` 0.5.18)

- Terminator kinds (`TerminatorKind`, `src/lower/java.rs::terminator_kind`):
  `return_statement` → `Return`, `break_statement` → `Break`,
  `continue_statement` → `Continue`, `throw_statement` → `Throw`.
- Block-kind containers (`IrNode::in_block`; the same containers
  `docs/clone-detection.md`'s Java clone-candidate list names): `block` (a
  method/constructor body, or a nested `{ … }` scope), `constructor_body`.
- `IrNode::is_hoisted_or_type_only` is always `false` for Java — it has no
  equivalent exemption.

## Lowering: JS/TS

- Terminator kinds: identical node-kind literals to Java's own table —
  `return_statement`, `break_statement`, `continue_statement`,
  `throw_statement`.
- Block-kind containers: `statement_block` only — deliberately not
  `program`, the top-level module scope, which is never itself a braced
  block. (`*-UNREACHABLE-AFTER-RETURN`'s own file-root check restores
  exactly that one exception, so top-level statements after a terminator
  are still scanned even though `program` never sets `in_block`.)
- `IrNode::is_hoisted_or_type_only` is `true` for `function_declaration`,
  `generator_function_declaration` (hoisted), `type_alias_declaration`,
  `interface_declaration` (type-only).

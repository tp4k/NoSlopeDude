# Cyclomatic complexity (CC) rules

CC is computed from the pinned Tree-sitter grammars, normalized into this
scanner's own IR (`src/ir/mod.rs`) before any counting happens — never from
a hand-rolled tokenizer or regex line counter (the scanner "uses existing
parsers rather than implementing language grammars"). `CC = 1 + decision
points`, where a decision point is any `IrNode` inside a callable's own
`body_span` whose `decision: Option<DecisionKind>` field is `Some`,
excluding any nested callable's own span (a lambda or arrow inside a method
is its own, separately counted `IrCallable`).

This document does **not** claim numerical equivalence to the scb-check
paper's Python CC/SLOC scores — the formula (`mass`, `erosion`) is retained
verbatim, but language-specific counting and verbosity rules are documented
here so results stay interpretable on their own terms.

## Decision constructs (`DecisionKind`, D7)

Every `IrNode` either lowering produces carries `decision: Option<DecisionKind>`
(`src/ir/mod.rs`) — one shared enum instead of two per-grammar string
tables. Each variant adds exactly 1 to its callable's CC:

| `DecisionKind` | What it marks |
|---|---|
| `Branch` | an `if` |
| `Loop` | any loop form — `for`, a Java enhanced-`for`, a JS/TS `for…in`/`for…of` (reported as the same node kind, see Lowering below), `while`, `do` |
| `Case` | a non-`default` `switch`/`case` label |
| `Catch` | a `catch` clause |
| `Ternary` | `a ? b : c` |
| `And` | `&&` |
| `Or` | `\|\|` — JS/TS also maps `??` here (see Lowering below) rather than adding a new variant |

Every other construct — an `else` branch, a `finally` clause, a `default`
label, the JS/TS optional-chain operator `?.`, a labeled statement, a
`yield` expression — has `decision: None` and adds 0. The exact node kinds
this covers are named per language under Lowering below.

## Callables (`IrCallable`, D8 — what gets its own CC, SLOC and mass)

`IrCallable` (`src/ir/mod.rs`) is one entry per callable-kind node that has
a body, in document order: its own declaration `span`, its `body_span` (the
subtree CC and SLOC are measured over), and D10's resolved `name`. A
callable-kind node with no body (an interface/abstract method, a TS
signature) never becomes an `IrCallable` at all, so it contributes no SLOC,
CC or mass. Which node kinds are callables, and how each one finds its own
body, is named per language under Lowering below.

A callable whose `span` shares any byte with a `DamageSpan` (`DamageKind`,
`Span::intersects`) is excluded fail-closed (`lower::cascade_exclusions`,
`lower::prune_damage`): it never enters `IrFile::callables` at all, so it
contributes no CC, SLOC, mass, scanned lines, clone candidates or findings,
and a pruned damage subtree contributes nothing to any metric, rule or
clone candidate either.

A nested callable (a lambda or arrow inside a method, for example) is its
own, separately counted `IrCallable`: the enclosing callable's SLOC and CC
walk excludes the nested callable's own declaration `span` (`IrCallable::span`,
`metrics::nested_callable_spans`), so no source line and no decision point
is counted twice, and total mass is conserved.

D10 naming: the node's own `name` field; else the name read off an
enclosing name-carrying construct (named per language under Lowering
below); else `<anonymous>@<line>`.

## Executable SLOC (`IrNode::executable`, D11)

A callable's SLOC is the count of distinct 1-based source lines within its
`body_span` that contain at least one leaf `IrNode` (no children) with
`executable == true`. `executable` comes from `exec_lines::is_executable_leaf`,
which both lowerings call rather than restating: a leaf counts when it is
named (`IrNode::is_named`) and is not a comment (`IrNode::is_comment`). A
line holding only anonymous delimiters (`{`, `}`, `;`, `)`), only a comment,
or nothing at all, does not count. This is IR-derived, not a textual/regex
line count.

A bare `break`, `continue` or `return` (no expression) is a documented
exception: its own keyword and `;` are anonymous leaves with no named leaf
beneath them, so the rule above would otherwise drop it entirely. A
`break`/`continue`/`return` node (`TerminatorKind::Break`/`Continue`/
`Return`, not `Throw`) with zero **named** children counts as its own
executable line (`exec_lines::is_bare_control_flow`).

`scanned source lines` (used elsewhere as the verbosity denominator) is a
different, file-level quantity: the same per-leaf rule applied across an
entire successfully parsed file's IR tree, so it also covers lines outside
every callable (imports, field and type declarations) — it is not the sum
of every callable's SLOC in that file.

## Mass and erosion (D13, verbatim)

```
mass    = cc as f64 * (sloc as f64).sqrt()
erosion = (Σ mass of callables with cc > 10) / (Σ mass of all callables)
```

`erosion` is `0.0` when the total mass is `0.0` (including when there are no
callables at all). The `10` threshold is fixed and unrelated to
`--min-clone-lines`'s own default of `10`.

### Worked example

`tests/fixtures/metrics/erosion/HighComplexity.java` has one callable,
`compute`, with a branch (with an `And`), three loops (a `for`, a `while`,
a `do`), a catch, a ternary, two `switch` statements (3 non-`default` `case`
labels, so 3 `Case` decisions) and an `Or`:

```
cc   = 1 (base) + 11 (Branch, And, Loop×3, Catch, Ternary, Case×3, Or) = 12
sloc = 9   // the 9 executable lines inside compute's body
mass = 12 * sqrt(9) = 12 * 3.0 = 36.0
```

`tests/fixtures/metrics/erosion/LowComplexity.js` has one callable,
`lowComplexity`, with a branch, three loops (a `for`, a `while`, a `for…of`
— reported as the same `Loop`-mapped node kind as `for…in`, see Lowering
below), a ternary, an `And` and one `Case`:

```
cc   = 1 (base) + 7 (Branch, Loop×3, Ternary, And, Case) = 8
sloc = 9
mass = 8 * sqrt(9) = 8 * 3.0 = 24.0
```

Scanning `tests/fixtures/metrics` (which also contains
`__tests__/` fixtures excluded by the default test globs, and
`broken/Broken.ts`, a deliberately malformed file that fails to parse and
marks the scan `incomplete` per D18):

```
total mass = 36.0 + 24.0 = 60.0
erosion    = 36.0 / 60.0 = 0.6
```

The top-25 table's first row is `compute` (`erosion/HighComplexity.java`),
with `cc=12`, `sloc=9`, `mass=36.0`.

## Lowering: Java (`tree-sitter-java-orchard` 0.5.18)

`src/lower/java.rs::decision_kind` is the only place a Java grammar string
feeds `DecisionKind`:

| Grammar node kind | `DecisionKind` |
|---|---|
| `if_statement` | `Branch` |
| `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement` | `Loop` |
| a non-`default` `switch_label` | `Case` |
| `catch_clause` | `Catch` |
| `ternary_expression` | `Ternary` |
| `binary_expression` with operator `&&` | `And` |
| `binary_expression` with operator `\|\|` | `Or` |

`switch_label` is the operative decision node under **both** switch shapes:
the colon form (`case 1:`, a direct child of a `switch_block_statement_group`)
and the arrow form (`case 1 ->`, a required child of a `switch_rule`).
`switch_rule` itself carries no `DecisionKind` — the `switch_label` nested
inside it already does, so both forms count identically without a second
match arm. A `switch_label` is `default` when its own first child's kind is
literally `"default"` (covers both `default:` and `default ->`).
`tests/metrics.rs::test_switch_case_labels_count_each_default_excluded`
checks both switch shapes score `cc == 3`.

`finally_clause` and the `else` branch of an `if_statement` carry no
`DecisionKind` at all.

Callable kinds (D8): `method_declaration`, `constructor_declaration`,
`compact_constructor_declaration`, `static_initializer`, `lambda_expression`.
Each exposes its body through the `body` field, except `static_initializer`,
whose direct `block` child carries no field name.

D10 name resolution, when the node has no own `name` field: the enclosing
`variable_declarator`'s `name` field, `pair`'s `key` field, or
`assignment_expression`'s `left` field.

A Java interface or abstract `method_declaration` with no `block` child is
not a callable and contributes no SLOC, CC or mass.

## Lowering: JS/TS

`src/lower/jsts.rs::decision_kind` is the only place a JS/TS grammar string
feeds `DecisionKind`:

| Grammar node kind | `DecisionKind` |
|---|---|
| `if_statement` | `Branch` |
| `for_statement`, `for_in_statement`, `while_statement`, `do_statement` | `Loop` |
| `switch_case` (never `switch_default`) | `Case` |
| `catch_clause` | `Catch` |
| `ternary_expression` | `Ternary` |
| `binary_expression` with operator `&&` | `And` |
| `binary_expression` with operator `\|\|` or `??` | `Or` |

`for_in_statement` is the grammar's own node kind for **both** `for…in` and
`for…of` loops — there is no separate for-of kind.

`else_clause`, `finally_clause`, `switch_default`, the optional-chain
operator `?.` (`optional_chain`), `labeled_statement` and `yield_expression`
carry no `DecisionKind` at all.

Callable kinds (D8): `function_declaration`, `generator_function_declaration`,
`function_expression`, `arrow_function`, `method_definition`. Each exposes
its body through the `body` field.

A TS `function_signature` or `abstract_method_signature` (no body block) is
not a callable and contributes no SLOC, CC or mass.

D10 name resolution, when the node has no own `name` field: the enclosing
`variable_declarator`'s `name` field, `pair`'s `key` field, or
`assignment_expression`'s `left` field.

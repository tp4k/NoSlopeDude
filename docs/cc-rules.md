# Cyclomatic complexity (CC) rules

CC is computed from the pinned Tree-sitter grammars, never from a hand-rolled
tokenizer or regex line counter (the scanner "uses existing parsers rather
than implementing language grammars"). `CC = 1 + decision points` counted in
a callable's own body, excluding any nested callable's span (a lambda or
arrow inside a method is its own, separately counted callable).

This document does **not** claim numerical equivalence to the scb-check
paper's Python CC/SLOC scores — the formula (`mass`, `erosion`) is retained
verbatim, but language-specific counting and verbosity rules are documented
here so results stay interpretable on their own terms.

## Decision constructs by language

### Java

Each of the following adds exactly 1:

- `if_statement`
- `for_statement`
- `enhanced_for_statement`
- `while_statement`
- `do_statement`
- `catch_clause`
- `ternary_expression`
- each `switch_label` that is not `default` (colon form: `case 1:`)
- each `switch_rule` whose label is not `default` (arrow form: `case 1 ->`)
- each `binary_expression` whose operator is `&&` or `||`

Adds 0: `else` / the `else` branch of an `if_statement`, `finally_clause`,
the `default:` label, and the `default ->` rule.

### JS/TS

Each of the following adds exactly 1:

- `if_statement`
- `for_statement`
- `for_in_statement` — the grammar reports **both** `for…in` and `for…of`
  loops as `for_in_statement`; there is no separate for-of kind
- `while_statement`
- `do_statement`
- `catch_clause`
- `ternary_expression`
- each `switch_case` (never `switch_default`)
- each `binary_expression` whose operator is `&&`, `||` or `??`

Adds 0: `else_clause`, `finally_clause`, `switch_default`, the optional-chain
operator `?.` (`optional_chain`), `labeled_statement`, `yield_expression`.

## Callables (what gets its own CC, SLOC and mass)

- **Java** — `method_declaration`, `constructor_declaration`,
  `compact_constructor_declaration`, `static_initializer`,
  `lambda_expression`.
- **JS/TS** — `function_declaration`, `generator_function_declaration`,
  `function_expression`, `arrow_function`, `method_definition`.

A node of one of these kinds with no body block (a TS `function_signature`
or `abstract_method_signature`, or a Java interface/abstract
`method_declaration` with no `block`) is not a callable and contributes no
SLOC, CC or mass.

A nested callable (a lambda or arrow inside a method, for example) is its
own, separately counted callable: the enclosing callable's SLOC and CC
exclude the nested callable's span, so no source line and no decision point
is counted twice, and total mass is conserved.

Callable naming: the node's own `name` field; else the name from an
enclosing `variable_declarator`, `pair` or `assignment_expression`; else
`<anonymous>@<line>`.

## Executable SLOC (D11)

A callable's SLOC is the count of distinct 1-based source lines within its
body span that contain at least one leaf token belonging to a *named* node
and that is not a comment. A line holding only anonymous delimiters (`{`,
`}`, `;`, `)`), only a comment, or nothing at all, does not count. This is
grammar-derived, not a textual/regex line count.

A bare `break;`, `continue;` or `return;` (no expression) is a documented
exception: its own keyword and `;` are anonymous tokens with no named leaf
beneath them, so the rule above would otherwise drop it entirely. A
`break_statement`, `continue_statement` or `return_statement` node with zero
named children counts as its own executable line.

`scanned source lines` (used elsewhere as the verbosity denominator) is a
different, file-level quantity: the same per-line rule applied across an
entire successfully parsed file, so it also covers lines outside every
callable (imports, field and type declarations) — it is not the sum of
every callable's SLOC in that file.

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
`compute`, with an `if` (with `&&`), a `for`, a `while`, a `do`, a `catch`,
a ternary, two single-case `switch` statements (3 non-default `case` labels
total) and an `||`:

```
cc   = 1 (base) + 11 (if, &&, for, while, do, catch, ternary, case×3, ||) = 12
sloc = 9   // the 9 executable lines inside compute's body
mass = 12 * sqrt(9) = 12 * 3.0 = 36.0
```

`tests/fixtures/metrics/erosion/LowComplexity.js` has one callable,
`lowComplexity`, with an `if`, a `for`, a `while`, a `for…of` (reported as
`for_in_statement`), a ternary, an `&&` and one `case`:

```
cc   = 1 (base) + 7 (if, for, while, for_in, ternary, && , case) = 8
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

# The Rust lowering (`tree-sitter-rust` 0.24.2)

Post-v1 row PV-1. `src/lower/rust.rs` is the third lowering. Like the Java and
JS/TS tables it is the only place, besides `src/lower/mod.rs`, that reads a
Rust grammar string; every analyzer reads the shared IR. `nsd` gates its own
`.rs` sources with it, and a scan reports a third language family, `rust`.

Versions: `lower::RUST_LOWERING_VERSION` (1) and
`profile::RUST_GRAMMAR_VERSION` (pinned against `Cargo.lock` by
`tests/profile.rs::test_version_constants_match_cargo_lock`) both enter the
`nsd-v1` measurement fingerprint, which therefore moves. The profile name
stays `nsd-v1` and `schema_version` stays `1`: the new `rust` keys are
additive (see `docs/measurement-profile.md`).

## Mapping

| IR concept | tree-sitter-rust node(s) | Rule |
|---|---|---|
| language | extension `rs` | `LanguageFamily::Rust`, `Grammar::Rust`, JSON key `rust` |
| callable | `function_item` with a `body`; `closure_expression` | `CallableKind::RustFunction`, `RustClosure`. A trait `function_signature_item` (no body), `macro_rules!` and const/static items are not callables |
| name | `name` field of a `function_item`; for a closure, the `pattern` of the `let_declaration` it is the value of, else anonymous | the closure fallback mirrors Java lambda / JS arrow naming |
| signature | the `type` text of every typed `parameter`, whitespace-normalized | `self` and untyped closure parameters add nothing |
| owner | `mod_item` is a `Namespace`; `impl_item` is a `NamedType` named `Trait for Type` or `Type` (whitespace-normalized); `struct_item`, `enum_item` and `trait_item` are `NamedType`; an enclosing function is a `Callable` | two `impl X for T` blocks with a `fn fmt` have distinct identities |
| Branch | `if_expression` (including `if let`); a `let_declaration` with an `alternative` (let-else); a match-arm guard | +1 each |
| Loop | `while_expression` (including `while let`), `for_expression` | `loop_expression` has no predicate and is **not** a decision |
| Case | each `match_arm` whose pattern is not the bare wildcard `_` | mirrors Java's non-default `switch_label`; a guarded `_ if c` arm is still the bare wildcard: it adds no Case, only its guard's +1 Branch |
| And / Or | `binary_expression` with `&&` / `\|\|` | |
| `?` | `try_expression` | not a decision |
| Catch / Ternary | none | Rust has neither |
| terminators | `return_expression` Return, `break_expression` Break, `continue_expression` Continue | no Throw; `panic!` is a macro, not a terminator |
| blocks, clone statements | `block`; the clone statements are its named, non-comment direct children, tail expression included | `in_catch_body` is always false |
| comments | `line_comment`, `block_comment`, the `doc_comment` text and the `outer`/`inner_doc_comment_marker` leaves inside them | `exec_lines::is_comment_kind`; a block comment's marker is a childless named leaf, so it is listed explicitly |
| executable lines | as Java: named leaves are executable, so statements and expressions count; the whole subtree of an `attribute_item` or `inner_attribute_item` does not | an attribute identifier is a named leaf, so it is silenced by subtree (`Classification::silences_executable_subtree`) |
| bare control flow | `return_expression`, `break_expression`, `continue_expression` join `BARE_CONTROL_FLOW_KINDS` | a bare `return;` line stays executable |
| damage | `ERROR` / `MISSING` give `DamageKind::Unclassified` | as Java |
| hoisted / type-only | always false | |
| rules | none | no Rust rule IDs; `ALL_RULE_IDS` keeps its six |

## Limitations

- A `macro_invocation` token tree is an opaque run of leaves. Its tokens feed
  clone fingerprints and its lines count as executable, but a decision inside
  a macro argument (`println!("{}", if x { 1 } else { 2 })`) is not counted.
- The six built-in rules do not run on Rust: the family reports CC, SLOC,
  mass, erosion and clone groups, and a verbosity score whose flagged lines come only from redundant clone
  occurrences, because no rule finding contributes.
- `async` blocks, `unsafe` blocks and `const` blocks are plain expressions;
  only `function_item` and `closure_expression` are callables.

The mapping is pinned by `tests/rust_lowering.rs`.

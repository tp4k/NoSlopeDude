# The `nsd-v1` measurement profile

`src/profile.rs` defines the profile's name, pinned parser versions, input
fields and fingerprint function. The fingerprint covers the IR version, three
lowering versions, five grammar versions, the tree-sitter runtime version,
the rule catalog and the effective clone threshold.

This document covers the profile's shape and how to re-derive its fingerprint.
The report and cache consume `profile::fingerprint`.

## What `PROFILE_NAME` names

`profile::PROFILE_NAME = "nsd-v1"` identifies *which* profile a fingerprint
was computed under — the scanner's current measurement algorithm as a
whole (D-IR, the six D22 rules, the D23 verbosity score, `min_clone_lines`
as the one user-tunable input), as opposed to any future `nsd-v2`. It is
not itself a fingerprint input.

## The eleven fingerprint inputs, and where each comes from

`profile::MeasurementProfileInputs` carries exactly these eleven fields, and
`profile::fingerprint` pushes each one into the digest in this order:

| Input | Source | Why it is here |
|---|---|---|
| `ir_version` | `ir::IR_VERSION` | The normalized IR's own shape (`src/ir/mod.rs`'s doc comment: bump whenever `IrNode`'s shape changes what a downstream consumer reads off it) |
| `java_lowering_version` | `lower::JAVA_LOWERING_VERSION` | What the Java lowering classifies a parse tree into (`src/lower/mod.rs`) |
| `jsts_lowering_version` | `lower::JSTS_LOWERING_VERSION` | The JS/TS counterpart |
| `rust_lowering_version` | `lower::RUST_LOWERING_VERSION` | The Rust counterpart (`docs/rust-lowering.md`) |
| `tree_sitter_runtime_version` | `profile::TREE_SITTER_RUNTIME_VERSION`, pinned against `Cargo.lock`'s `tree-sitter` package | The parsing runtime itself, distinct from either grammar: it belongs in the fingerprint but is not a grammar |
| `java_grammar_version` | `profile::JAVA_GRAMMAR_VERSION`, pinned against `Cargo.lock`'s `tree-sitter-java-orchard` package | A lowering can be correct and still be handed different input by a grammar bump (*Settled decisions* → *Measurement profile*) |
| `javascript_grammar_version` | `profile::JAVASCRIPT_GRAMMAR_VERSION`, pinned against `Cargo.lock`'s `tree-sitter-javascript` package | Same reasoning, JS half |
| `typescript_grammar_version` | `profile::TYPESCRIPT_GRAMMAR_VERSION`, pinned against `Cargo.lock`'s `tree-sitter-typescript` package | Same reasoning, TS half |
| `rust_grammar_version` | `profile::RUST_GRAMMAR_VERSION`, pinned against `Cargo.lock`'s `tree-sitter-rust` package | Same reasoning, Rust |
| `rule_catalog` | `rules::ALL_RULE_IDS` | Which D22 rules ran; adding, removing or renaming a rule id changes what a finding count means |
| `min_clone_lines` | `model::DEFAULT_MIN_CLONE_LINES` for the default profile, or a scan's own `ScanSettings::min_clone_lines` for a non-default one | The effective threshold changes both the measurement fingerprint and the cache key |

`MeasurementProfileInputs::current()` builds the default-configuration set
by reading every field off its own live source of truth above — it cannot
drift from those constants, because it does not re-declare them.

Not a fingerprint input: `tree_sitter::LANGUAGE_VERSION` /
`MIN_COMPATIBLE_LANGUAGE_VERSION` (ABI numbers, not the crate version the
plan means by "the `tree-sitter` runtime version").

## The fingerprint itself

`profile::fingerprint` is a versioned BLAKE3 digest
(`src/hashing.rs::Digest`, under the `"measurement-profile"` family
prefix — its own hash domain, distinct from `src/golden.rs`'s
`"golden-digest-body"` prefix and from `src/clones/mod.rs`'s per-language
prefixes), formatted `blake3:{32 lowercase hex digits}`, the same shape
`src/golden.rs::body_blake3` commits.

`tests/profile.rs::test_nsd_v1_fingerprint_is_frozen` pins the
default-configuration fingerprint to a committed hex literal; identical
inputs always yield identical output
(`test_identical_inputs_give_identical_fingerprints`), and every one of the
eleven inputs above independently moves it
(`test_each_input_changes_the_fingerprint`).

## The bump rule

A future change to any of the eleven inputs above **must** move the frozen
literal in `tests/profile.rs::test_nsd_v1_fingerprint_is_frozen` — the test
fails first, loudly, rather than letting a downstream cache key or golden
digest silently keep comparing against stale data. When that happens:

1. Confirm the change is an intentional profile change, not an accidental
   one (e.g. a routine dependency bump that happens to touch a pinned
   grammar or runtime version).
2. Update the moved constant(s) in `src/profile.rs` if the change is a
   version bump `test_version_constants_match_cargo_lock` would otherwise
   catch as stale.
3. Recompute the literal and record *why* it moved in the commit that
   updates it — the same way `IR_VERSION`'s and the lowering versions' own
   doc comments already name the change that bumped them.

`test_version_constants_match_cargo_lock` is the standing guard against
dependency-version drift: a `Cargo.lock` version bump for one of the four
grammars or the `tree-sitter` runtime that nobody remembers to mirror into
`src/profile.rs`.

## Moving the fingerprint's hash never renames the profile

`PROFILE_NAME` names the measurement algorithm as a whole (see above), not
any one fingerprint value. When one of the eleven inputs moves — for example
M1-7 bumping `ir::IR_VERSION` for `IrCallable`'s new identity-bearing
fields, or M3-4 bumping it again for `IrFile::excluded_callables` — only the frozen hex literal in
`tests/profile.rs::test_nsd_v1_fingerprint_is_frozen` changes; the profile
keeps being called `nsd-v1`, because it is still the same algorithm being
re-measured, not a new one.

## Not covered

Three other kinds of change leave the fingerprint unchanged, and nothing
fails:

(a) A lowering or IR behaviour change made without bumping the matching
    `IR_VERSION` / `*_LOWERING_VERSION` constant — the author must bump it
    by hand; the fingerprint has no way to detect that the behaviour
    behind an unchanged version number moved.

(b) A rule-logic change that keeps its id unchanged in
    `rules::ALL_RULE_IDS` — only the ids are hashed (`src/profile.rs:103-105`),
    not the rules' own logic, so this has no fingerprint input today.

(c) A clone-algorithm change, e.g. `src/clones/mod.rs`'s
    `MIN_CANDIDATE_STATEMENTS` or its token normalization — this, too, has
    no fingerprint input today.

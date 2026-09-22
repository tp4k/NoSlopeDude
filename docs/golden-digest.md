# The `java-fixture-01` golden digest

`nsd-plan-final.md`'s A1 amendment ("*Golden is a digest*") picks
`java-fixture-01@c6671504…` as the one clean, complete recorded scan and
commits it as a **digest**, not the underlying `report.json`: that file
carries private source excerpts and an absolute checkout path and stays
outside this repository, in the `agent_slope` archive, forever. See
`nsd-plan-final.md`'s *Measured starting state* and *Settled decisions* →
*Golden fixture* for the decision; this document only covers the digest's
shape and how to re-derive it.

## What is committed

`tests/golden/java-fixture-01.digest.json` holds exactly these fields, and
nothing else:

| field | content |
|---|---|
| `label` | the neutral fixture name (`"java-fixture-01"`), replacing the archive's absolute checkout path |
| `language` | the fixture's language, from `nsd-plan-final.md`'s *Measured starting state* row |
| `authorship` | `"unknown"` — not inferred from the scan (`AGENTS.md`) |
| `revision_sha` | `scan.revision.sha`, the one checkout-independent identity the archive carries |
| `hash_version` | the `body_blake3` hash family's version tag |
| `scores` | per-language `erosion` and `verbosity` (`flagged_lines`, `scanned_lines`, `ratio`), verbatim from the archive |
| `findings_by_rule_id` | each rule ID's finding count |
| `clones` | `{ group_count, total_redundant_lines }` over the archive's duplicate groups |
| `skips_by_reason` | each skip reason's file count |
| `top25` | the top 25 callables as `[cc, sloc, mass]` triples, in the archive's own order — names and locations dropped |
| `body_blake3` | an algorithm-prefixed lowercase-hex BLAKE3 (`blake3:<32 hex chars>`) of the normalized report body |

No absolute path, private repository name, source excerpt, or callable name
appears anywhere in this file.

## Normalization

Before anything is read out of it, the archived `report.json` is normalized
in two steps (A1):

1. Every `excerpt` key is stripped, at every nesting depth (`findings[].location`,
   `duplicates[].locations[]`, and `top25[].location` all carry one).
2. `scan.target` (the archive's absolute checkout path) is replaced with the
   neutral `label` above.

`body_blake3` is a versioned BLAKE3 (`src/hashing.rs`'s `Digest`) over the
serialized bytes of the normalized report, so it changes if the archive's
content changes but never encodes the checkout path or any excerpt.

## A float-parsing correction

This crate's pinned `serde_json` release parses at least one real literal in
the `java-fixture-01` archive (`scores.java.verbosity.ratio`,
`0.009966469683777625`) one ULP off — it round-trips back out as
`0.009966469683777623` through a plain `serde_json::from_str::<Value>`, while
Rust's own `str::parse::<f64>` gets the same literal exactly right.
`serde_json`'s own fix for this (`float_roundtrip`, or `arbitrary_precision`)
requires a `Cargo.toml` feature edit outside this workstream's scope, so
`src/golden.rs`'s `parse_report` instead re-derives every float leaf this
digest reads (`scores.*.erosion`, `scores.*.verbosity.ratio`, `top25[].mass`)
directly from the archive's raw text, checks each recovered value against
what `serde_json` parsed (catching a misaligned search rather than silently
committing a wrong number), and only then hands the result to
`build_digest`. `src/golden.rs`'s own unit tests reproduce the exact literal
above as a regression test.

## Re-deriving the digest

The archive never enters this repository, so re-deriving it requires the
checkout it came from:

1. Resolve the archived report by matching `revision_sha` above against
   `scan.revision.sha` in `~/pet/agent_slope`'s scan outputs.
2. Run the reproducibility test with that path:

   ```
   NSD_ARCHIVED_REPORT=<resolved path> cargo test --test golden_digest
   ```

   `test_committed_digest_matches_the_archived_report` re-derives the digest
   from the archive via the same `parse_report` / `build_digest` pipeline and
   asserts it equals the committed file. Without `NSD_ARCHIVED_REPORT` set,
   this test (and `test_gate_is_reported_pending_when_the_archive_is_absent`)
   report the reproducibility leg as **pending**, not passing — a missing
   archive never silently reads as green.
3. `test_committed_digest_carries_no_paths_names_or_excerpts` runs
   unconditionally, with no archive needed, and checks the committed file
   itself for forbidden keys and absolute-path-shaped strings.

# The `java-fixture-01` golden digest

`java-fixture-01@c6671504…` is the clean, complete reference scan. The
repository commits its sanitized **digest**, not the underlying `report.json`:
that report contains private source excerpts and an absolute checkout path.
This document describes the digest and how to re-derive it.

## What is committed

`tests/golden/java-fixture-01.digest.json` holds exactly these fields, and
nothing else:

| field | content |
|---|---|
| `label` | the neutral fixture name (`"java-fixture-01"`), replacing the archive's absolute checkout path |
| `language` | the fixture's recorded language |
| `authorship` | `"unknown"` — not inferred from the scan (`AGENTS.md`) |
| `revision_sha` | `scan.revision.sha`, the one checkout-independent identity the archive carries |
| `hash_version` | the `body_blake3` hash family's version tag |
| `scores` | per-language `erosion` and `verbosity` (`flagged_lines`, `scanned_lines`, `ratio`), verbatim from the archive |
| `findings_by_rule_id` | each rule ID's finding count |
| `clones` | `{ group_count, total_redundant_lines }` over the archive's duplicate groups |
| `skips_by_reason` | each skip reason's file count |
| `top25` | the top 25 callables as `[cc, sloc, mass]` triples, in the archive's own order — names and locations dropped |
| `body_blake3` | `blake3:` followed by 32 lowercase hex characters — the leading 16 bytes of a versioned, domain-separated BLAKE3 digest over the normalized report body, **not** a plain BLAKE3 hash of the body bytes (see *A BLAKE3 framing note* below) |

No absolute path, private repository name, source excerpt, or callable name
appears anywhere in this file.

## Normalization

Before anything is read out of it, the archived `report.json` is normalized
in two steps (A1):

1. Every `excerpt` key is stripped, at every nesting depth (`findings[].location`,
   `duplicates[].locations[]`, and `top25[].location` all carry one).
2. `scan.target` (the archive's absolute checkout path) is replaced with the
   neutral `label` above.

### A BLAKE3 framing note

`body_blake3` is **not** a plain BLAKE3 hash of the normalized body's
serialized bytes — it is `src/hashing.rs`'s versioned, domain-separated
`Digest`, and reproducing it requires the exact same framing:

1. The current `HASH_VERSION` byte (`1` as of WS-3) is fed first, as the
   hash input's very first byte.
2. The `"golden-digest-body"` family prefix is pushed next, keeping this
   digest's hash space separate from `src/clones/mod.rs`'s per-language
   family prefixes even on identical input bytes.
3. The normalized report body's serialized bytes are pushed last.

Steps 2 and 3 are each framed by their own 8-byte little-endian length
prefix ahead of the bytes themselves, so no ambiguity can arise between
where one pushed slice ends and the next begins. `body_blake3` itself is
only the *leading 16 bytes* of the resulting BLAKE3 output, read as a
little-endian `u128` and formatted as the 32 lowercase hex characters after
the `blake3:` prefix — not the full 32-byte BLAKE3 digest. It still changes
if the archive's content changes and never encodes the checkout path or any
excerpt, but it cannot be reproduced by hashing the body bytes alone with a
generic BLAKE3 tool.

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
   report the reproducibility leg as **pending**, not passing — no equality
   is ever asserted on a missing archive, so it can never invent a pass.
   That said, a plain `cargo test` still reports the pending run as Cargo's
   ordinary `ok`, indistinguishable from a verified one in the summary
   line; set `NSD_REQUIRE_ARCHIVE_VERIFIED=1` alongside the archive path to
   turn a still-missing archive into a panic instead, for a CI job (once
   wired) that has the archive and wants to demand it.
3. `assert_digest_carries_no_paths_names_or_excerpts` runs unconditionally,
   with no archive needed, and is called by both
   `test_committed_digest_carries_no_paths_names_or_excerpts` and
   `test_nsd_v1_digest_carries_no_paths_names_or_excerpts`, each against its
   own committed file (the M0b digest and the `nsd-v1` digest). It asserts:
   the forbidden keys and the widened path-shape scan (any `/`, `\`, or `~`
   in any object key or string value, exempting only the exact `language`
   value, not just a leading `/`); the exact eleven-key top-level set; the
   exact `label`, `authorship`, `revision_sha`, and `language` values;
   `body_blake3`'s `blake3:` + 32-lowercase-hex-char shape; `scores`' exact
   language set (`{overall, java, js_ts}` for the M0b file; the `nsd-v1`
   file adds PV-1's `rust`), per-language `{erosion,
   verbosity}` key shape and `verbosity`'s key shape (`{flagged_lines,
   ratio, scanned_lines}` for the M0b file; the `nsd-v1` file adds M1-8's
   `complete` and `unanalyzed_lines`); and that every leaf under `scores`,
   `clones`, `findings_by_rule_id`, `skips_by_reason` and `top25` is
   numeric, except the `nsd-v1` file's boolean `scores.*.verbosity.complete`.

## The `nsd-v1` digest

`tests/golden/java-fixture-01.nsd-v1.digest.json` is the current Java
end-to-end golden. The older `java-fixture-01.digest.json` remains as a
historical fixture because `test_committed_digest_matches_the_archived_report`
still checks it against the archive.

`tests/golden_digest.rs::test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01`
re-scans the archive's own recorded target with its own
recorded settings (`include_tests`, `exclude`, `min_clone_lines`), exactly
as the retired M0b-8c strict leg did, and asserts the result equals this
file. It first asserts the live report's `scan.revision.sha` equals the
M0b digest's `revision_sha`, `scan.revision.dirty` is `null` (a local scan
computes no dirty flag), the archive checkout itself is clean per
`git2::Repository::statuses` with untracked files included (that checkout is
the user's own trusted archive, and git2 runs no filter driver), and
top-level `incomplete` is `false` -- so a re-capture against the wrong or
a dirty checkout fails loudly instead of silently drifting.

The archive path is read only from `NSD_ARCHIVED_REPORT` at invocation
time, exactly like the M0b check; without it the test prints a PENDING
notice, same as above.

The archive checkout's cleanliness check opens the recipe target with
`git2::Repository::discover`, so a `recipe.target` that is a subdirectory of
the checkout passes it; a target outside any repository still fails loudly.

Re-capturing (implementer-only; overwrites the committed file) sets a
second, separate opt-in alongside the archive path:

```
NSD_ARCHIVED_REPORT=… NSD_GOLDEN_CAPTURE=1 cargo test --test golden_digest
```

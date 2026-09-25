# Implementation status (living document)

This file tracks **which plan steps are actually done, where, and on what
evidence**. It is the single status surface for the work described in
[`nsd-plan-final.md`](../nsd-plan-final.md) (authoritative) and
[`nsd-plan-implementation.md`](../nsd-plan-implementation.md) (execution
detail). It does not restate either plan — every row links back to the step it
tracks by the plan's own numbering.

Every agent that finishes a piece of work updates its rows here in the same
round; see *How to update this file* at the bottom and the matching rule in
[`AGENTS.md`](../AGENTS.md).

Last updated: 2026-09-24, from `main@a6b5037`, `feat/m1-snapshots@f23804f`,
and `feat/m0c-grammar@1a665df` (M0c's WS-1..WS-5 and their follow-up fixes
landed; not yet merged to `main`).

## Legend

| Mark | Meaning |
|---|---|
| `[x]` | Done: landed on the named branch, with a named check that passes |
| `[~]` | In progress: part of the step has landed, the rest is named in *Evidence* |
| `[!]` | Blocked or pending: implemented as far as possible, gated on something outside the repository |
| `[ ]` | Not started |

A row is only `[x]` when a test, script or recorded measurement in this
repository proves it. "The code exists" is `[~]`, not `[x]`.

## Branch map

| Branch / worktree | HEAD | Holds |
|---|---|---|
| `main` (`~/pet/nsd`) | `a6b5037` | M0a and M0b, fast-forwarded from `feat/m0a-import` on 2026-09-24. The integration branch |
| `feat/m0a-import` (deleted) | merged into `main` at `a6b5037` | Was the M0a/M0b working branch; its worktree `~/pet/nsd-m0a-import` was removed with it |
| `feat/m1-snapshots` (`~/pet/nsd-m1-snapshots`) | `f23804f`, branched from `c5267c0` | M1–M2 snapshots, config, diff, rename detection, Git-backed discovery |

`feat/m1-snapshots` is **not** merged and has diverged: it forked at `c5267c0`
(mid-M0b, right after the cc/SLOC IR retarget), so it is missing 45 commits
that `main` now has — the clone and rules retargets, all of WS-6's salvage
work, the `ir_isolation`/`salvage` suites, and this file. It must merge `main`
before its own M1 rows can be called green against the current M0b baseline.

Test state on `main@a6b5037`: `cargo test` is green, 173 passed /
0 failed / 1 ignored across 17 suites (local Homebrew toolchain; MSRV 1.90 is
declared, not exercised — A4).

## M0a — Port and consolidate

Plan: `nsd-plan-final.md` *M0a — Port and consolidate*, steps 1–3.

| ID | Step | Status | Branch | Evidence |
|---|---|---|---|---|
| M0a-1 | Selective `git archive` import from `agent_slope@912ec7a`, rename crate/bin/CLI/docs to `nsd`, record provenance, keep root plans authoritative | `[x]` | `main` | `37829cb`, `dfff3e5`, `06b5ef5`; `tests/cli.rs::test_binary_name_is_nsd`, `tests/report_html.rs` `<title>`/`<h1>` pin (`6bc362f`) |
| M0a-2 | Consolidate the three D11 executable-line implementations into one shared function | `[x]` | `main` | `be3a586` → `src/exec_lines.rs`; unit tests `57ada8b`; ledger row closed `67a6f51` |
| M0a-3 | Replace `DefaultHasher` with versioned BLAKE3 before anything persists a fingerprint | `[x]` | `main` | `5d4a79f`, `2c49e7b` (`src/hashing.rs`, `RunFingerprint` rewritten, SipHash streams dropped); unit tests `78b3506` |
| M0a-4 | Golden digest (A1) for `java-fixture-01`, numbers-and-hashes only, no private excerpts or paths | `[x]` | `main` | `a92d5f2`, `f1bfc1b`, `5a9ac01`; `tests/golden_digest.rs`, `docs/golden-digest.md`; privacy test widened `1e60aa6`, `60ed9e1` |
| M0a-5 | Declare `rust-version = "1.90"` (A4) | `[x]` | `main` | `Cargo.toml:5` |

Not a plan step, recorded for accuracy: `src/golden.rs` works around a
`serde_json` float-rounding bug via raw-text recovery (`5e0f3d1`, guarded by
`MAX_RELATIVE_CORRECTION`, `49e6597`).

## M0b — IR, on the existing grammars

Plan: `nsd-plan-final.md` *M0b — IR, on the existing grammars*, steps 4–8.
Grammars deliberately unchanged here: `tree-sitter` 0.25.10,
`tree-sitter-java` 0.23.5, `tree-sitter-javascript` 0.25.0,
`tree-sitter-typescript` 0.23.2.

| ID | Step | Status | Branch | Evidence |
|---|---|---|---|---|
| M0b-4 | Prototype clone lowering before freezing IR shape; `normalized_statement_tokens` from the IR must reproduce today's clone groups exactly | `[~]` | `main` | Reproduction is proven — `tests/clones.rs::test_ir_backed_groups_match_the_pre_ir_groups_on_every_fixture` (`edbf309`), plus `test_statement_children_come_from_ir_block_membership` and `test_fingerprint_is_stable_across_two_runs_of_the_ir_path`. `[~]` because it was proven *after* the IR shape landed (`671060e` → `0401481`), not by a prototype ahead of it; the exact-group requirement itself is met on every suite fixture, and on `java-fixture-01` only through the pending leg in M0b-8 |
| M0b-5 | Define `src/ir/`; Java and JS/TS lowerings in `src/lower/` as plain functions over tree-sitter trees, both emitting typed damage spans | `[x]` | `main` | `c48fa70` (red), `671060e` (green), built per file in the pipeline `7d132ef`; six remaining IR capabilities `9400a23`/`5038473`; hardening `91475eb` (iterative `Drop`), `6ab7c44` (`Span` → u32), `e332181`; `tests/ir_lowering.rs`, `tests/ir_parity.rs` |
| M0b-6 | Retarget `cc`, D11/SLOC, clones and the six rules onto the IR | `[x]` | `main` | cc/SLOC `c5267c0`; D8/D10/block classification `a839679`; clones `0401481`; six rules + verbosity `9e3e7dd`. No analyzer names a grammar node kind: `tests/ir_isolation.rs::test_no_analyzer_names_a_grammar_node_kind`, with its own positive control (`f92c33c`) |
| M0b-7 | Error-span salvage and the `SkipReason` split, IR-level and language-agnostic | `[x]` | `main` | `aacca37` (salvage per callable instead of whole-file drop), redesigned as one coherent pass `35508d9`, bare-damage-span ancestors `75ec24d`; `tests/salvage.rs` (14 tests incl. the quadratic-blowup guard, `5bf9c34`); `SkipReason` at `src/model.rs:77` |
| M0b-8a | Measurement-neutrality gate on the ten suites' fixtures: byte-identical `report.json`, pre-IR vs IR, same fixed checkout and identical settings | `[x]` | `main` | `tests/neutrality.rs` + `scripts/neutrality_gate.sh` (`4ee26c2`, `2424019`, `502a358`); clean/malformed corpus partition `b2290ea`/`1ccfeac`; baselines recaptured `08e95ae`, `687a86b`, `8af5134`; documented in `docs/ir-neutrality.md` |
| M0b-8b | Declared-delta exception: salvage and the `SkipReason` split change malformed fixtures only, asserted explicitly, unaffected measurements identical | `[x]` | `main` | `bbdf429`, `f5655ad`, `4ec3d94` (`/skipped_files` delta tightened to an exact length); `docs/ir-neutrality.md` *Declared deltas* |
| M0b-8c | Strict leg: byte-identical against the archived full `report.json` for `java-fixture-01@c6671504…`, run locally from the checkout path that report records | `[x]` | `feat/m0c-grammar` | **PASS**, run by the coordinator on `main@3dd9ae2` against the private archive with `NSD_REQUIRE_ARCHIVE_VERIFIED=1`: `test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report` reported a byte-identical 2,613,042-byte `report.json`; the tampered-archive negative control (a one-byte-flipped copy of the archive) correctly failed the same comparison. Last run at `tree-sitter-java` 0.23.5, before this workstream's grammar swap (M0c-9); the test and its pending twin were then retired at M0c-10, see `docs/ir-neutrality.md` *The `java-fixture-01` strict leg (retired at M0c-10)*. No archive path or excerpt is recorded here or anywhere in this repository |

## M0c — Grammar swap

Plan: `nsd-plan-final.md` *M0c — Grammar swap, now a one-lowering change*,
steps 9–12. M0b-8c cleared; the grammar swap (M0c-9) and its acceptance gate
(M0c-10) are done on `feat/m0c-grammar`, not yet merged to `main`. M0c-11
through M0c-13 are separate workstreams' scope and remain as recorded below.

| ID | Step | Status | Branch | Evidence |
|---|---|---|---|---|
| M0c-9 | Swap to `tree-sitter-java-orchard` 0.5.18, bump `tree-sitter` to 0.27.0 | `[x]` | `feat/m0c-grammar` | `9b36269`. `Cargo.toml`/`Cargo.lock` now pin `tree-sitter` 0.27.0 / `tree-sitter-java-orchard` 0.5.18, no `tree-sitter-java` (`grep -c '^name = "tree-sitter-java"$' Cargo.lock` is 0); JS/TS grammars unchanged. `ir::IR_VERSION` 2→3, `lower::JAVA_LOWERING_VERSION` 2→3 (`JSTS_LOWERING_VERSION` unchanged), `DamageKind::JavaVarargsAnnotation` removed. `cargo test` green, 173+ tests across 20 suites |
| M0c-10 | Quantify the re-baseline: re-run `scripts/perf_scan.sh` on the pinned Spring+Angular fixture, record parse-failure movement (55 Java → 0 required, 3 TS `using` remain) and the Java SLOC/mass/erosion delta, **explained** on ≥2 hand-checked callables | `[x]` | `feat/m0c-grammar` | `735cf5f`, `743df6f`, `4a4a039`; gate strengthened round 2 in `e7301c3`. Gate cleared: `tests/grammar_gate.rs` (new, env-gated on `NSD_PERF_FIXTURE`, reads unfiltered `PipelineOutput::parse_failures` since `report.json`'s `skipped_files` filters `SyntaxError` out) shows 0 Java + exactly 3 TS `SyntaxError` failures on the pinned Spring+Angular fixture. Delta quantified and explained in `docs/measurements.md` *M0c-10*: the sole mechanism is fail-closed damage-exclusion lifting (a varargs-annotation parse fix), **not** orchard's new `modifier`/`visibility` node types as hypothesized — those wrapper nodes always have exactly one child and so can never satisfy `is_executable_leaf`, proven on a real parse dump and a corpus-wide sweep. Two hand-checked Spring callables (`ClassUtils#getMethodIfAvailable`, `ReflectionUtils#findMethod`) with exact before/after CC/SLOC/mass in the same section. Neutrality baselines recaptured/reverted and explained in `docs/ir-neutrality.md` *M0c-10* |
| M0c-11 | Re-baseline fixture expectations; update the three `docs/*.md` contracts to describe IR kinds, not grammar node names | `[x]` | `feat/m0c-grammar` | Re-baseline (M0c-9/M0c-10): `9b36269`, `735cf5f`, `743df6f`, `4a4a039`, `e7301c3`. Contract docs rewritten in IR vocabulary (`DecisionKind`, `IrCallable`, `IrNode::executable`/`terminator`/`in_block`/`in_catch_body`/`is_clone_statement`), every remaining grammar node-kind string confined to a "Lowering" subsection naming `tree-sitter-java-orchard` 0.5.18 for Java: `f0fbb32` (`docs/cc-rules.md`), `c0f2017` (`docs/wasteful-rules.md`), `c96fcdc` (`docs/clone-detection.md`). No semantic change: `cargo test` green (`tests/rules.rs::test_every_rule_id_is_documented` included), no `src/` file touched. `docs/cc-rules.md`'s pre-existing `switch_rule` mention (a possible doc/lowering disagreement flagged in this stream's brief) checked against `src/lower/java.rs::decision_kind` and found not to be a behavioral gap — `switch_rule`'s own required `switch_label` child already supplies the `Case` decision for the arrow form, so both switch forms score identically (`tests/metrics.rs::test_switch_case_labels_count_each_default_excluded` passes unmodified); documented in `docs/cc-rules.md`'s Java Lowering subsection |
| M0c-12 | Freeze `nsd-v1`: fingerprint over IR version, both lowering versions, three grammar versions, `tree-sitter` runtime version, rule catalog, effective clone configuration incl. `min_clone_lines` | `[x]` | `feat/m0c-grammar` | `cc8e88e` (red), `2759220` (green), `ce28756` (doc). `src/profile.rs`: `pub mod profile` with `PROFILE_NAME = "nsd-v1"`, four version constants (`tree-sitter` runtime, `tree-sitter-java-orchard`, `tree-sitter-javascript`, `tree-sitter-typescript`), `MeasurementProfileInputs::current()` (reads `ir::IR_VERSION`, `lower::JAVA_LOWERING_VERSION`/`JSTS_LOWERING_VERSION`, `rules::ALL_RULE_IDS`, `model::DEFAULT_MIN_CLONE_LINES`), and `fingerprint()` under its own `"measurement-profile"` `hashing::Digest` family prefix. `tests/profile.rs` (5 tests): each of the nine inputs independently moves the fingerprint, identical inputs give identical fingerprints, the four version constants are pinned against `Cargo.lock` by a plain-string test (no new dependency), and the default-configuration fingerprint is pinned to a committed hex literal. No `report.json` field, no cache, no CLI flag — see `docs/measurement-profile.md`. `cargo test` green, 178 passed / 0 failed / 1 ignored across 20 suites; `cargo fmt --check` and `cargo build --release` (0 warnings) also pass |
| M0c-13 | Clear the cheap ledger rows named in the plan: `erosion` returning `-0.0`, `BTreeSet<usize>` → sorted `Vec`, the `rposition` and `always_returns` survivors, the `LanguageFamily::JsTs` guard at `src/rules/mod.rs:339`, `tests/report_html.rs`'s `#L1-L1` anchor, and `Callable::end_line` | `[x]` | `feat/m0c-grammar` | All seven rows cleared from [`deferred-work.md`](deferred-work.md), each with a new regression test (`tests/metrics.rs`, `tests/rules.rs`, `tests/verbosity.rs`, `tests/report_json.rs`, `tests/report_html.rs`). `Callable::end_line` landed last (commit `0c28b06`), after M0b-8c's own strict-leg gate (already `[x]`/retired above), with the clean neutrality baseline recaptured (commit `4603d34`) and documented in [`ir-neutrality.md`](ir-neutrality.md) *M0c-13*; `report-format.md`'s "Top-25 span" section updated to match. `cargo test` green. Mutation-survivor test evidence: `f4106ec` (`tests/metrics.rs::test_erosion_with_no_eroded_mass_is_positive_zero`), `0c703e3` (`tests/rules.rs::test_unreachable_after_return_starts_after_the_first_terminator`, `test_redundant_else_fires_on_a_multi_statement_returning_branch`, `test_java_top_level_terminator_is_not_an_unreachable_container`), `6e44be5` (`tests/verbosity.rs::test_executable_lines_are_sorted_and_distinct`), `0c28b06` (`tests/report_json.rs::test_top25_span_covers_the_whole_callable`, `tests/report_html.rs::test_links_point_at_the_scanned_revision`). Round 2 (review findings): `153dc96` (`tests/report_json.rs::test_top25_span_covers_the_whole_callable`'s exact-span pins, closing the `end_line: start_line + 1` mutant the `>=` loop alone had missed), `a781f0a` (`tests/verbosity.rs::test_executable_lines_are_sorted_and_distinct`'s literal already-sorted `Vec`, no longer sorting its own input), `070e2fb`/`b211daf` (doc and test-comment corrections, no behavior change) |
| M0c-14 | Post-swap `nsd-v1` golden digest for `java-fixture-01`, captured from a live scan after orchard (WS-1), the freeze (WS-3) and `end_line`/`-0.0` (WS-4) all landed; env-gated live-scan check; M0b digest kept byte-identical as history (D15) | `[x]` | `feat/m0c-grammar` | `beee958` (red: `tests/golden_digest.rs::test_nsd_v1_digest_matches_a_live_scan_of_java_fixture_01` and four supporting tests, failing on the missing file), `4659fb9` (green: `tests/golden/java-fixture-01.nsd-v1.digest.json` captured against the archive with `NSD_REQUIRE_ARCHIVE_VERIFIED=1`, revision sha/dirty/incomplete checked first; negative control on `total_redundant_lines` correctly failed, reverted before commit), `b21b22e` (doc: [`golden-digest.md`](golden-digest.md) *The `nsd-v1` digest* section, including the per-field delta table). `cargo test --test golden_digest` (19 tests) green both with and without the archive; `cargo test` full suite green; `tests/golden/java-fixture-01.digest.json` (M0b) unchanged (`git diff 3dd9ae2 --` empty) |

## M1–M2 — Snapshots, diffs, identity

Plan: `nsd-plan-final.md` *M1–M2*, plus `nsd-plan-implementation.md`
*Implementation sequence* step 3. All work below is on `feat/m1-snapshots`,
which still needs the 45 commits `main` has and it does not (see *Branch map*);
its rows are green against its own fork point, not against current M0b.

| ID | Deliverable | Status | Branch | Evidence |
|---|---|---|---|---|
| M1-1 | `git2`-backed commit, index and worktree snapshots over repository-relative UTF-8 bytes; `--staged` reads the index, never the dirty worktree | `[x]` | `feat/m1-snapshots` | `a17822b`, `1126f1e` (lazy bounded reads, ODB-header sizing), `fb30bf0`; `tests/git_snapshots.rs`, `b9d0da4` |
| M1-2 | Git-backed discovery from trees/indexes, independent of candidate `.gitignore`; worktree mode overlays tracked changes and eligible untracked files under base policy | `[x]` | `feat/m1-snapshots` | `28023f4` (red), `e938134` (green); `tests/git_discovery.rs`; immutable-exclusion glob compile failure fails closed `2c19e31` |
| M1-3 | Merge-base resolution with an actionable shallow-clone error (`NSD-G101`) | `[x]` | `feat/m1-snapshots` | `src/git/mergebase.rs` (`a17822b`); `134c2d2` asserts the `NSD-G101` prefix and a non-vacuous version needle; `fbedfa2` keeps the git code through `ConfigError::from_git` |
| M1-4 | Centralize line mapping as one shared primitive for parse errors, findings, suppressions, callables and clone attribution | `[~]` | `feat/m1-snapshots` | Primitive landed with the central diff (`2bce3f5`), full line-map coverage `22eba67`. `[~]` until the five consumers exist — findings, suppressions and clone attribution are M3–M5 |
| M1-5 | Handle additions, deletions, modifications, renames, symlinks, submodules, invalid UTF-8 and large files deterministically | `[x]` | `feat/m1-snapshots` | `2bce3f5`; over-ceiling worktree entries hashed not emptied `e8bc84e`, typechange kept as one delta `f23804f`, non-UTF-8 similarity header parsed as bytes `e247638`, modes read from tree/index entries `8fb8457`, symlink/gitlink coverage `58f0110`/`22eba67` |
| M1-6 | Rename detection: libgit2, fixed 50% similarity, configured threshold asserted plus one clear rename and one clear non-rename (A6) | `[x]` | `feat/m1-snapshots` | `1b2b199` (per-delta similarity), `ee1cfee`, `f3bea70` (exact rename = full similarity), `dfe73cc` (`max_size` capped at the ceiling), `b968aaf`/`2820c9c` (ignores `diff.renames`/`diff.renamelimit`) |
| M1-7 | Replace `<anonymous>@<line>` with a line-independent callable identity | `[ ]` | — | Nothing on either branch |
| M1-8 | Report mapped parser gaps and coverage explicitly; ratios use *analyzed* executable lines and carry completeness metadata | `[ ]` | — | Related: M0b-7's salvage removed the per-file `SyntaxError` provenance; restoring it is a [`deferred-work.md`](deferred-work.md) row explicitly deferred to this milestone |
| M2-1 | Strict repository-root `nsd.yml` parsing, all `C102` cases, incl. `measurement.min_clone_lines` | `[x]` | `feat/m1-snapshots` | `44eb0fc` (red), `c1728cc` (green), blank/null include `a7c9b5c`, blank/comment-only globs `b7d00ef`; `tests/config.rs`; deps `serde_yaml_ng` 0.10, `git2` 0.21 |
| M2-2 | Base-policy trust: trusted `--config` replaces repository policy; candidate config validated through `C101` but cannot affect its own check | `[ ]` | — | — |
| M2-3 | Callable matching across snapshots | `[ ]` | — | Prerequisite for E101/E102 in M3 |
| M2-4 | Deterministic snapshot IDs | `[ ]` | — | No `snapshot_id` in `src/git/` |

## M3–M5 — Policy, suppressions, clones, cache

Plan: `nsd-plan-final.md` *M3–M5*. **Not started** — no branch holds any of it.

| ID | Deliverable | Status |
|---|---|---|
| M3-1 | E101/E102 after callable matching; deletions and improvements never fail | `[ ]` |
| M3-2 | V101 after diff-aware finding matching, so line-only movement is not a regression | `[ ]` |
| M3-3 | S101/S102: new suppressions via diff mapping and underlying finding matches; moving a finding with its unchanged directive is not new, transferring it to an unmatched finding raises `S101` (A9); invalid/unused raise `S102` | `[ ]` |
| M4-1 | Required clone-index coverage for unchanged included files while `V102` is enabled: size, encoding, capability failures raise `A102` (A10); mapped legacy parse-damage tolerance preserved | `[ ]` |
| M4-2 | V102 comparison: candidate changed occurrences vs unchanged base content and other candidate changes; exclude the replaced base version of a modified path; maximal-group reduction before emission; move mapping first (A3) | `[ ]` |
| M5-1 | Per-blob cache under `$GIT_COMMON_DIR/nsd/cache/v1`, keyed blob OID + language/grammar + measurement fingerprint; atomic writes; corruption or version mismatch is a recomputable miss | `[ ]` |
| M5-2 | Best-effort LRU eviction, 1 GiB / 30-day default | `[ ]` |
| M5-3 | Exit aggregation over all twelve diagnostics | `[ ]` |

## M6–M7 — Output, CI hardening

Plan: `nsd-plan-final.md` *M6–M7*. **Not started.** The imported engine
already emits a scan-scoped `report.json` and standalone HTML, but none of it
has been brought to the v1 canonical contract (`schema_version: 1`,
`result_scope`, snapshot IDs, per-reason skip counts, check scope).

| ID | Deliverable | Status |
|---|---|---|
| M6-1 | Canonical report `schema_version: 1` distinguishing `scan` and `check` scopes | `[ ]` |
| M6-2 | Canonical JSON: complete entity/diagnostic set, stable ordering, fixed numeric serialization, repo-relative paths, snapshot IDs, configuration and measurement fingerprints, per-reason skip counts, no timestamps/absolute paths/excerpts | `[ ]` |
| M6-3 | Terminal (50) and agent (30) renderers with deterministic omitted counts; exit status always reflects the complete result | `[ ]` |
| M6-4 | Scan HTML standalone, each excerpt capped at 20 lines / 4 KiB with an explicit truncation marker | `[~]` — HTML exists (`src/report/html.rs`), caps and truncation marker not implemented |
| M7-1 | Built-in exclusions incl. nested `.git` checkouts; `scan` excludes test/fixture/`e2e`/QA paths unless requested, `check` has no default test exclusion | `[~]` — `src/discover.rs` has the scan-side set; nested-checkout exclusion is on `feat/m1-snapshots` for Git-backed discovery only |
| M7-2 | `--allow-new-suppressions` plumbing, full-scan path, DoD sweep; documented copyable pre-commit and CI commands | `[ ]` |

## Standing gates and known debt

- **M0b-8c cleared** (PASS, see its row above) and M0c-9/M0c-10 are done on
  `feat/m0c-grammar`, not yet merged to `main`. The strict leg that gated it
  is now retired (`docs/ir-neutrality.md` *The `java-fixture-01` strict leg
  (retired at M0c-10)*); parser-invariance is no longer the right question
  once the parser itself has changed.
- **`feat/m1-snapshots` is 45 commits behind `main`.** Merge `main` into it
  before treating its rows as green against current M0b, and re-run
  `scripts/neutrality_gate.sh` afterwards. It is the only unmerged branch.
- [`deferred-work.md`](deferred-work.md) is the companion ledger: consciously
  postponed items, not defects. Rows named in M0c-13 are scheduled; the rest
  are unscheduled.
- Recorded measurements live in [`measurements.md`](measurements.md); its
  "Known caveats" paragraph is stale after salvage (a ledger row tracks it).

## How to update this file

1. **Update it in the round that does the work**, not afterwards. If a
   commit closes part of a row, the same commit or the round's final commit
   updates the row.
2. **Only your own rows.** Do not change a row another workstream owns, and
   do not re-mark a row someone else marked. If you believe another row is
   wrong, say so in your report instead of editing it.
3. **`[x]` requires evidence in this repository.** Put the commit SHA and the
   test, script or document that proves it in *Evidence*. A row with no named
   check stays `[~]`.
4. **Never mark a row `[x]` on a red check.** If the work landed but its check
   fails or could not be run, use `[~]` or `[!]` and say why in *Evidence*.
5. **A missing fixture or archive is `[!]`, never `[x]`.** Record what is
   missing and what would close it.
6. **New scope gets a new row**, with an ID that extends its milestone
   (`M4-3`, `M2-5`, …). Do not renumber existing rows — other documents and
   run reports cite these IDs.
7. **Refresh the header line** (`Last updated:` with the date and the branch
   HEADs you verified against) whenever you touch the file.
8. **Deferred work goes to [`deferred-work.md`](deferred-work.md)**, not here.
   This file tracks plan steps; that one tracks what was consciously
   postponed.

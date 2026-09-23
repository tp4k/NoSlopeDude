# The measurement-neutrality gate

`nsd-plan-final.md`, M0b item 8, requires that retargeting the scanner onto
the new IR produce a **byte-identical** `report.json` on every fixture the
retarget does not deliberately change. This document covers the gate that
proves it: what it compares, how it is run, and the declared-delta list
WS-6 populates for the salvage / `SkipReason` split item 8 explicitly
permits (see *Declared deltas* below).

Two mechanisms, kept deliberately separate:

- **`tests/neutrality.rs`** (the always-on gate): scans a fixed, copied
  fixture corpus with fixed settings and asserts the rendered `report.json`
  equals a committed pre-IR baseline byte-for-byte (after normalizing the
  one volatile field). It never shells out to git, needs no worktree, no
  network and no private archive, and runs as part of `cargo test`.
- **`scripts/neutrality_gate.sh`** (the operator-run tool): builds the
  pinned pre-IR commit in a detached worktree and the current `HEAD`, scans
  the same copied corpus with each binary, and diffs their `report.json`
  output directly against each other rather than against a committed file.
  This is the tool a human runs to re-capture the committed baselines (via
  its `--capture` mode), or to double-check the gate against git history
  rather than a static file.

## What is committed

`tests/golden/neutrality/`:

| file | corpus |
|---|---|
| `clean.report.json` | all of `tests/fixtures/` **minus** the three malformed sources below and **minus** `CLEAN_CORPUS_ONLY_EXCLUSIONS` — intended to be zero parse failures, so any measurement delta here is an unambiguous IR fidelity bug (see *Known gap* below: two pre-existing `tests/fixtures/ir/` fixtures currently break this invariant) |
| `malformed.report.json` | exactly `tests/fixtures/metrics/broken/Broken.ts`, `tests/fixtures/rules/broken/Broken.java`, `tests/fixtures/report/src/Broken.java` — parse failures the fixture salvage/`SkipReason` work is expected to eventually touch |

WS-6 also introduced `tests/fixtures/salvage/Mixed.java` and `Mixed.ts`
(`tests/salvage.rs`'s own fixtures, one clean callable and one damaged
callable each). They are excluded from `clean.report.json`'s walk via
`tests/neutrality.rs::CLEAN_CORPUS_ONLY_EXCLUSIONS`, a list kept
deliberately separate from `MALFORMED_CORPUS_SOURCES`: folding a new
fixture into `MALFORMED_CORPUS_SOURCES` would also grow
`malformed.report.json`'s own scanned corpus past what its baseline was
captured from — a corpus-*size* change (a `/top25` length mismatch, a
`scanned_lines` total moved by a file the baseline never saw at all), which
`DECLARED_DELTAS` has no way to express as a per-field delta.
`CLEAN_CORPUS_ONLY_EXCLUSIONS` only ever shrinks the clean-corpus walk;
`Mixed.java`/`Mixed.ts` never enter either scanned corpus, and needed no
baseline re-capture at all, since neither baseline was ever captured with
them present.

Both corpora are **copied** at test time (and at script run time) into a
fresh temporary directory, mirroring each source's path relative to
`tests/fixtures/` rather than flattening it, so directory names such as
`__tests__` or `broken` are classified by the D16 default exclusions and by
`include_tests`'s `TEST_GLOBS` exactly as a real scan target would see them.
Both scans run with `include_tests: true` (`--include-tests` on the CLI),
per the plan's decision that the neutrality corpus is not itself subject to
the test-path exclusion. The copy step fails loudly on a basename collision
that would otherwise silently overwrite an already-copied file. The fixture
files themselves are never edited in place, and nothing in this repository
writes into `tests/fixtures/` as a side effect of running the gate.

Every corpus source is also asserted present in the pipeline's own
discovery output (`discovered` ∪ `skipped`), so a source lost by the corpus
copy (e.g. by a silent overwrite, or a directory the D16 exclusions
swallow whole) fails the test directly rather than merely producing a
smaller-than-expected diff.

### Re-capturing after the corpus changes

The clean corpus is defined by **membership** of `tests/fixtures/**` minus
the three malformed sources, not by an explicit file list, so adding or
deleting any fixture obsoletes the committed `clean.report.json` — the
comparison in `test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline`
will fail on the new file's presence alone, with no measurement bug
involved. The procedure to move the baseline honestly, rather than paper
over the failure, is:

1. **Proof A**, ground truth: `bash scripts/neutrality_gate.sh` with no
   arguments. It builds the pinned pre-IR commit in a detached worktree and
   the current `HEAD`, and diffs their `report.json` directly against each
   other on the same copied corpus, so it is immune to corpus drift by
   construction. It must print `NEUTRALITY: identical on 2 corpora`; if it
   reports a divergence, stop — a capture on a diverged tree is a silent
   re-baseline, not an explained one.
2. **Proof B**, a drift bound: a control scan of the clean corpus minus the
   changed/added fixtures, compared byte-for-byte against the *currently
   committed* `clean.report.json`. This proves no pre-existing fixture's
   numbers moved, so the only thing the re-capture can introduce is the
   new file(s)' presence.
3. Run `bash scripts/neutrality_gate.sh --capture` to rewrite the golden
   files from `HEAD`'s own render — never hand-edit or reconstruct the JSON.
4. Check the resulting diff is bounded to exactly what corpus growth
   predicts: the affected `/scores/*` aggregates (denominators and any
   ratios/erosion derived from them) and, if the new file's callables rank
   into the top 25, the displaced `/top25` rows — nothing else.

`tests/fixtures/salvage/` (WS-6) was the fixture addition this note
originally anticipated triggering this procedure. It ended up not needing
it: `Mixed.java`/`Mixed.ts` are excluded from the clean-corpus walk via
`CLEAN_CORPUS_ONLY_EXCLUSIONS` instead (see the table above) rather than
folded into `MALFORMED_CORPUS_SOURCES`, so neither committed baseline ever
needed to change on their account. WS-6's own brief separately forbids
regenerating either baseline under `tests/golden/neutrality/` at all
(`AGENTS.md`, *Verification*: "never substitute invented results or
silently re-baseline an unexplained difference"), which this procedure
remains available for a human operator to run deliberately, with the two
proofs below, if a *genuine* corpus-composition change is ever needed.

### Known gap: two pre-existing damaged fixtures under `tests/fixtures/ir/`

WS-6 discovered, empirically (a diagnostic scan of the full `tests/fixtures/`
tree lists every parse failure across the whole corpus), that
`tests/fixtures/ir/JavaVarargsAnnotation.java` and
`tests/fixtures/ir/JsxUnterminatedEntity.tsx` (WS-1-owned, read-only to this
stream; both are hand-crafted fixtures for `tests/ir_lowering.rs`'s own
direct damage-kind assertions) also carry a `SyntaxError`, and are walked
into the clean corpus by `clean_corpus_sources()`'s membership rule.
(A third, `tests/fixtures/ir/TsUsingParameter.ts`, also carries a
`SyntaxError` but nets zero surviving scanned lines, so it does not itself
move a number — see below.)

Before salvage, a `SyntaxError` file was dropped wholesale regardless of
which corpus scanned it, so these two contributed nothing to
`clean.report.json`'s scores and the original decision-1 malformed list
(predating WS-6) never needed to name them; the committed baseline itself,
however, already carries their *skip bookkeeping* — three
`skipped_files` entries (`parse_syntax_error`) and `incomplete: true` — since
`build_skipped_files` rendered every parse failure unconditionally at
capture time.

Salvage means a `SyntaxError` file is lowered like any other file now, so
leaving these fixtures on the clean corpus's walk moves two numbers on a
target this gate has no mechanism to declare a delta on at all
(`test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline` is an
unconditional `assert_eq!`, with no `DECLARED_DELTAS`-style escape hatch):
confirmed by running the clean-corpus test with only this stream's `src/`
production changes applied (no test or fixture edits) — `+2` scanned lines
overall (`+1` java from `JavaVarargsAnnotation.java`'s surviving class
wrapper, `+1` js_ts from `JsxUnterminatedEntity.tsx`'s), and all three
fixtures' `skipped_files` entries disappear (`incomplete` itself stays
`true` on both sides, since `metrics`/`rules` incomplete tracking still
sees the residual `ParseFailure`s regardless).

Excluding these two from the clean corpus (the same
`CLEAN_CORPUS_ONLY_EXCLUSIONS` mechanism `Mixed.java`/`Mixed.ts` use) does
not resolve this: the committed `clean.report.json` baseline was captured
*with* them present, so removing them instead swaps which direction the
byte-identity assertion fails in (the three `skipped_files` entries and the
`+2` scanned lines both move, just the other way) rather than eliminating
the divergence. The only way to make `test_clean_corpus_report_is_byte_
identical_to_the_pre_ir_baseline` pass again, either way, is an authorized
re-capture of `clean.report.json` under the corrected corpus composition
via the Proof A/B procedure above — which is exactly the WS-2-owned
baseline regeneration this stream's brief forbids without asking first.
This is left as an open question for the coordinator; see the round's
implementer report.

## Normalization

Two normalization mechanisms exist, at two different levels:

- **Raw-text normalization** (`normalize_raw_text`), used for the
  byte-identity comparison itself: the corpus's own absolute
  temporary-directory path is replaced with the fixed label
  `<neutrality-corpus>` in the raw `report.json` text `nsd` wrote to disk —
  asserting first that it occurs exactly once. This compares the actual
  bytes the binary renders (struct declaration order, exact whitespace),
  not a re-serialized `serde_json::Value` (whose maps use alphabetical key
  order and would not detect a real serialization regression).
- **Value-level normalization** (`normalize`), used only for the
  malformed-corpus's declared-deltas comparison and for
  `tests/neutrality.rs::test_normalization_replaces_only_the_scan_target`,
  which asserts that normalizing at the `Value` level touches only
  `/scan/target` by diffing the normalized report against the unnormalized
  one.

`scan.revision` needs no normalization: the copied corpus lives outside any
Git work tree, so `src/target.rs`'s `local_git_revision` always resolves it
to the same constant block, `{"sha": null, "dirty": null,
"unavailable_reason": "not_a_git_repository"}`.
`tests/neutrality.rs::test_corpus_copy_is_outside_any_git_work_tree` pins
this.

`scripts/neutrality_gate.sh` scans the *same* corpus directory with both
binaries it builds, so `scan.target` is identical in both reports by
construction and needs no normalization there either.

## Declared deltas

`tests/neutrality.rs`'s `DECLARED_DELTAS` constant is a list of JSON
pointers (e.g. `/skipped_files/0/reason`) that the malformed-corpus
comparison is permitted to differ on, applied by `diff_paths`/`walk_diff`
against both a changed leaf and a key present on only one side. WS-6
populates it with exactly three pointers, one by one:

- **`/skipped_files`** — a `SyntaxError` file is no longer a whole-file
  skip (`src/report/mod.rs::build_skipped_files` now filters out
  `ParseFailureReason::SyntaxError`), so all three of
  `MALFORMED_CORPUS_SOURCES`'s fixtures' entries disappear from the
  rendered array: it shrinks from 3 entries to 0. `walk_diff` reports an
  array-length mismatch once, at the parent path, not per index, so a
  single pointer covers all three removals.
- **`/scores/overall/verbosity/scanned_lines`** and
  **`/scores/java/verbosity/scanned_lines`** — `rules/broken/Broken.java`
  and `report/src/Broken.java` each declare exactly one callable and it
  intersects the damage, so it stays unmeasured (fail-closed, D9); but
  D12's per-file scanned-line count is unconditioned on callable
  boundaries (`metrics::scan_file`'s own doc comment) and still walks each
  file's surviving class-wrapper lines around the pruned callable —
  previously `0` (the whole file was dropped before reaching `metrics` at
  all), now a small positive count per file.

`js_ts`'s own malformed fixture, `metrics/broken/Broken.ts`, needs **no**
declared delta: its damage happens to leave no surviving wrapper content
outside the one damaged callable, so `/scores/js_ts/verbosity/scanned_lines`
stays `0` on both sides. `flagged_lines` and every `ratio` field also stay
identical on both sides too (`0`, since `0/0` and `0/scanned_lines` are both
defined as `0.0` by `compute_verbosity`) and need no entry either — none of
the three fixtures' surviving wrapper lines trip any of the six rules, so
nothing moves the numerator.
`tests/neutrality.rs::test_diff_paths_suppresses_only_declared_deltas`
exercises the mechanism directly against hand-built JSON, with a non-empty
declared-deltas list, covering a changed leaf under a declared pointer, a
key added on only one side under a declared pointer, and a changed leaf
outside any declared pointer — independent of whatever `DECLARED_DELTAS`
holds in production.

## The `java-fixture-01` strict leg

Spec item 8 also names a byte-identity comparison against
`java-fixture-01`, the private archived report — separate from
`docs/golden-digest.md`'s lossy digest comparison (below).
`tests/neutrality.rs::test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report`
covers it, gated the same way `tests/golden_digest.rs` gates its own
archive comparison: `NSD_ARCHIVED_REPORT` (a path to the private archived
`report.json`, read only at test run time) resolves the leg; when unset,
the test prints a pending notice and passes, unless
`NSD_REQUIRE_ARCHIVE_VERIFIED` is also set, in which case it panics.
`test_java_fixture_01_strict_leg_is_reported_pending_when_the_archive_is_absent`
pins the pending path. When resolved, the archived report's own
`scan.{target,include_tests,exclude,min_clone_lines}` are read back out of
it to rebuild the exact `ScanSettings` used to produce it, a fresh scan is
run, and the two `report.json` texts are compared byte-for-byte. Neither
the archive's path nor any excerpt of its contents is ever written to this
repository.

## Running the gate

```
cargo test --test neutrality
```

runs the always-on comparison against the committed baselines; it needs
nothing beyond the checked-out repository (aside from the optional,
private-archive-gated `java-fixture-01` leg above, which is pending by
default).

```
bash scripts/neutrality_gate.sh
```

re-derives the comparison against git history instead of the committed
files: it builds the pinned pre-IR commit in a detached worktree, builds
`HEAD`, scans the same copied corpus with both, and prints
`NEUTRALITY: identical on <n> corpora` and exits `0` when they agree, or
`NEUTRALITY: <corpus> corpus diverged at <json pointer>` and exits `1` on
the first field that does not.

```
bash scripts/neutrality_gate.sh --capture
```

re-captures `tests/golden/neutrality/clean.report.json` and
`malformed.report.json` from the current `HEAD`, by running
`cargo test --test neutrality` with `NSD_NEUTRALITY_CAPTURE=1` set — the
same raw-text capture path the Rust harness's two corpus tests use, so the
committed baseline is guaranteed byte-identical to what the compare mode
would later read. It does not touch the pre-IR worktree at all.

## What this gate is not

Beyond the `java-fixture-01` strict byte-identity leg described above,
this gate does not cover the wider ten-suite private archive comparison or
`docs/measurements.md`'s perf fixture — those stay outside this repository
entirely (`AGENTS.md`, *Fixture privacy*) and are covered by
`docs/golden-digest.md` and `tests/golden_digest.rs`'s lossy digest
comparison instead. No private repository name, checkout path, source
excerpt, or name mapping appears anywhere in this document or in
`tests/golden/neutrality/`.

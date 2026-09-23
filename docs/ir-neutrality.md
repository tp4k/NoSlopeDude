# The measurement-neutrality gate

`nsd-plan-final.md`, M0b item 8, requires that retargeting the scanner onto
the new IR produce a **byte-identical** `report.json` on every fixture the
retarget does not deliberately change. This document covers the gate that
proves it: what it compares, how it is run, and the (currently empty)
declared-delta list WS-6 populates when it lands the salvage / `SkipReason`
split item 8 explicitly permits.

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
| `clean.report.json` | all of `tests/fixtures/` **minus** the three malformed sources below — zero parse failures, so any measurement delta here is an unambiguous IR fidelity bug |
| `malformed.report.json` | exactly `tests/fixtures/metrics/broken/Broken.ts`, `tests/fixtures/rules/broken/Broken.java`, `tests/fixtures/report/src/Broken.java` — parse failures the fixture salvage/`SkipReason` work is expected to eventually touch |

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
against both a changed leaf and a key present on only one side. It is
empty as of this stream: WS-2 captures the pre-IR baseline before any
analyzer is retargeted, so there is nothing yet to declare. WS-6 populates
it when the salvage and `SkipReason` split lands, and only for the
specific fields item 8 names — every other measurement on the malformed
corpus must stay identical.
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

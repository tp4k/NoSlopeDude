# The measurement-neutrality gate

`nsd-plan-final.md`, M0b item 8, requires that retargeting the scanner onto
the new IR produce a **byte-identical** `report.json` on every fixture the
retarget does not deliberately change. This document covers the gate that
proves it: what it compares, how it is run, and the (currently empty)
declared-delta list WS-6 populates when it lands the salvage / `SkipReason`
split item 8 explicitly permits.

Two mechanisms, kept deliberately separate:

- **`tests/neutrality.rs`** (the always-on gate): scans a fixed, copied
  fixture corpus with fixed settings and asserts the normalized
  `report.json` equals a committed pre-IR baseline. It never shells out to
  git, needs no worktree, no network and no private archive, and runs as
  part of `cargo test`.
- **`scripts/neutrality_gate.sh`** (the operator-run tool): builds the
  pinned pre-IR commit in a detached worktree and the current `HEAD`, scans
  the same copied corpus with each binary, and diffs their `report.json`
  output directly against each other rather than against a committed file.
  This is the tool a human runs to re-capture the committed baselines, or
  to double-check the gate against git history rather than a static file.

## What is committed

`tests/golden/neutrality/`:

| file | corpus |
|---|---|
| `clean.report.json` | `tests/fixtures/rules/__tests__/{CleanJava.java,CleanJs.js,CleanTs.ts}` — zero parse failures, so any measurement delta here is an unambiguous IR fidelity bug |
| `malformed.report.json` | `tests/fixtures/rules/broken/{Broken.java,Good.java}` — one parse failure alongside one file that parses, the fixture salvage/`SkipReason` work is expected to eventually touch |

Both corpora are **copied** at test time (and at script run time) from the
files above into a fresh temporary directory, flattened under `src/` so no
source directory name (`__tests__`, `broken`) is mistaken for a test or
generated-code path by the D16 default exclusions. The fixture files
themselves are never edited in place, and nothing in this repository writes
into `tests/fixtures/` as a side effect of running the gate.

## Normalization

Exactly one field is normalized before either comparison: `scan.target`,
the corpus's own absolute temporary-directory path, replaced with the fixed
label `<neutrality-corpus>`. Nothing else is touched —
`tests/neutrality.rs::test_normalization_replaces_only_the_scan_target`
asserts this directly, by diffing the normalized report against the
unnormalized one and requiring the only difference to be `/scan/target`.

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
comparison is permitted to differ on. It is empty as of this stream: WS-2
captures the pre-IR baseline before any analyzer is retargeted, so there is
nothing yet to declare. WS-6 populates it when the salvage and `SkipReason`
split lands, and only for the specific fields item 8 names — every other
measurement on the malformed corpus must stay identical.

## Running the gate

```
cargo test --test neutrality
```

runs the always-on comparison against the committed baselines; it needs
nothing beyond the checked-out repository.

```
bash scripts/neutrality_gate.sh
```

re-derives the comparison against git history instead of the committed
files: it builds the pinned pre-IR commit in a detached worktree, builds
`HEAD`, scans the same copied corpus with both, and prints
`NEUTRALITY: identical on <n> corpora` and exits `0` when they agree, or
`NEUTRALITY: <corpus> corpus diverged at <json pointer>` and exits `1` on
the first field that does not.

## What this gate is not

It does not cover `java-fixture-01@c6671504…`, the ten-suite private
archive comparison, or `docs/measurements.md`'s perf fixture — those stay
outside this repository entirely (`AGENTS.md`, *Fixture privacy*) and are
covered by `docs/golden-digest.md` and `tests/golden_digest.rs` instead. No
private repository name, checkout path, source excerpt, or name mapping
appears anywhere in this document or in `tests/golden/neutrality/`.

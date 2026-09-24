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
| `clean.report.json` | all of `tests/fixtures/` **minus** the three malformed sources below — intended to be zero parse failures, so any measurement delta here is an unambiguous IR fidelity bug (see *Resolved* below: two damaged `tests/fixtures/ir/` fixtures and salvage's own two `tests/fixtures/salvage/` fixtures all carry a `SyntaxError` and are on this walk) |
| `malformed.report.json` | exactly `tests/fixtures/metrics/broken/Broken.ts`, `tests/fixtures/rules/broken/Broken.java`, `tests/fixtures/report/src/Broken.java` — parse failures the fixture salvage/`SkipReason` work is expected to eventually touch |

WS-6 also introduced `tests/fixtures/salvage/Mixed.java` and `Mixed.ts`
(`tests/salvage.rs`'s own fixtures, one clean callable and one damaged
callable each). An earlier round of this stream excluded them from
`clean.report.json`'s walk via a since-deleted
`tests/neutrality.rs::CLEAN_CORPUS_ONLY_EXCLUSIONS` list, kept separate
from `MALFORMED_CORPUS_SOURCES` for the reason given below — but that
left the two fixtures covered by *neither* Rust corpus test, which
Decision 20 does not authorize (see *Resolved*, second entry). They now
join the clean corpus like any other fixture, and `clean.report.json` has
been re-captured to account for them. Folding them into
`MALFORMED_CORPUS_SOURCES` instead remains the wrong fix: it would also
grow `malformed.report.json`'s own scanned corpus past what its baseline
was captured from — a corpus-*size* change (a `/top25` length mismatch, a
`scanned_lines` total moved by a file the baseline never saw at all),
which `DECLARED_DELTAS` has no way to express as a per-field delta.

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
originally anticipated triggering this procedure, and it did: `Mixed.java`
and `Mixed.ts` join the clean corpus (see the third *Resolved* entry
below) exactly as Decision 20 (`plan.md:1000`, `:1030`) prescribes — an
earlier round of this stream instead excluded them from the clean-corpus
walk via a `CLEAN_CORPUS_ONLY_EXCLUSIONS` list, which has since been
deleted. WS-6's own brief separately forbids regenerating either baseline
under `tests/golden/neutrality/` at all (`AGENTS.md`, *Verification*:
"never substitute invented results or silently re-baseline an unexplained
difference"), which this procedure remains available for a human operator
to run deliberately, with the two proofs below, if a *genuine*
corpus-composition change is ever needed.

### Resolved: two pre-existing damaged fixtures under `tests/fixtures/ir/`

WS-6 discovered, empirically (a diagnostic scan of the full `tests/fixtures/`
tree lists every parse failure across the whole corpus), that
`tests/fixtures/ir/JavaVarargsAnnotation.java` and
`tests/fixtures/ir/JsxUnterminatedEntity.tsx` (WS-1-owned, read-only to this
stream; both are hand-crafted fixtures for `tests/ir_lowering.rs`'s own
direct damage-kind assertions) also carry a `SyntaxError`, and are walked
into the clean corpus by `clean_corpus_sources()`'s membership rule.
(A third, `tests/fixtures/ir/TsUsingParameter.ts`, also carries a
`SyntaxError` but nets zero surviving scanned lines, so it does not itself
move a score, only its own `skipped_files` entry — see below.)

Before salvage, a `SyntaxError` file was dropped wholesale regardless of
which corpus scanned it, so these two contributed nothing to
`clean.report.json`'s scores and the original decision-1 malformed list
(predating WS-6) never needed to name them; the committed baseline itself,
however, already carried their *skip bookkeeping* — three
`skipped_files` entries (`parse_syntax_error`) and `incomplete: true` — since
`build_skipped_files` rendered every parse failure unconditionally at
capture time.

Salvage means a `SyntaxError` file is lowered like any other file now, so
leaving these fixtures on the clean corpus's walk moved two numbers on a
target this gate has no mechanism to declare a delta on at all
(`test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline` is an
unconditional `assert_eq!`, with no `DECLARED_DELTAS`-style escape hatch).
This is authorized by `plan.md` Decision 20 (`plan.md:1000`, `:1030`),
which added `tests/golden/neutrality/` to WS-6's own file scope for exactly
this re-capture, the same way it did for WS-3 round 1's
`tests/fixtures/parity/` addition.

**Proof A** (`bash scripts/neutrality_gate.sh`, no args) diffs the current
`HEAD` binary against the pinned pre-IR commit directly, live, on the same
copied corpus — it never reads `tests/golden/neutrality/`, so it is a
second, independent proof of the same underlying claim the committed
baselines make, not a re-derivation of them. Its true, measured result,
established by running the identical command at three points in this
stream's history (recorded here rather than only in a round's own
implementer report, since a later reader of this file has no other way to
find it):

- **`1d1bb8a`** (`4cc8742^`, the commit immediately before WS-6 touched
  anything): exit 0, `NEUTRALITY: identical on 2 corpora`.
- **`8af5134`** (WS-6 round 2, `CLEAN_CORPUS_ONLY_EXCLUSIONS` still
  present): exit 1, `NEUTRALITY: clean corpus diverged at
  /scores/java/erosion`.
- **`1ccfeac`** (WS-6 round 3, after `CLEAN_CORPUS_ONLY_EXCLUSIONS` was
  deleted and `Mixed.java`/`Mixed.ts` joined the Rust harness's own clean
  corpus): still exit 1, still `NEUTRALITY: clean corpus diverged at
  /scores/java/erosion` — the identical first-divergence pointer.

This divergence is **WS-6-introduced, not pre-existing**: it was absent at
`1d1bb8a` and present at every commit inside this stream measured so far.
It is also, as of round 3, no longer explained by
`CLEAN_CORPUS_ONLY_EXCLUSIONS` (that mechanism is gone). A full diff
against the pinned pre-IR binary's own render (round 3, see the
implementer report's Verification section for the exact `jq` invocation)
names the actual, remaining cause precisely: five fixtures carry a
`SyntaxError` and are walked into this script's unfiltered "clean" corpus
(`tests/fixtures/ir/JavaVarargsAnnotation.java`,
`tests/fixtures/ir/JsxUnterminatedEntity.tsx`,
`tests/fixtures/ir/TsUsingParameter.ts`, `tests/fixtures/salvage/Mixed.java`,
`tests/fixtures/salvage/Mixed.ts`) — salvage measures all five partially
now, while the *pinned pre-IR binary* still drops each wholesale
(`skipped_files` shrinks from 13 entries to 8 on the `HEAD` side; the five
`parse_syntax_error` entries are exactly the five named above). This is
inherent to what this operator script computes — a live diff against a
fixed pre-commit-WS-6 binary — and is not itself a bug in the Rust
harness's own gate (which compares `HEAD` against a baseline captured
*from* `HEAD`'s own behavior, so it does not see this at all: both
`tests/neutrality.rs` corpus tests are green). It is an **open,
WS-6-caused gap** left for a teamlead/coordinator decision (move the
pinned pre-IR commit forward, or give this script its own
malformed/exclusion list mirroring `MALFORMED_CORPUS_SOURCES`) — not a
pre-existing one, and not this round's to silently work around.
`scripts/neutrality_gate.sh` is out of this stream's scope to edit and
stays untouched.

**Proof B**, a control diff against the *Rust harness's* own
`clean_corpus_sources()` (at round 2 time, this excluded
`CLEAN_CORPUS_ONLY_EXCLUSIONS`, since deleted — see the third *Resolved*
entry below), additionally excluding the two score-moving fixtures,
compared against the then-committed `clean.report.json`: confirmed the
only residual difference was
`/skipped_files` (from the third fixture, `TsUsingParameter.ts`, still
present and still losing its own now-universally-suppressed
`SyntaxError` skip entry, with zero score impact) — proving nothing else
in the clean corpus moved.

**Capture**: `NSD_NEUTRALITY_CAPTURE=1 bash scripts/neutrality_gate.sh
--capture` (delegating to `cargo test --test neutrality`) re-captured
`clean.report.json` from `HEAD`. The resulting diff is bounded to exactly
what the analysis above predicted: all three `skipped_files` entries
(`JavaVarargsAnnotation.java`, `JsxUnterminatedEntity.tsx`,
`TsUsingParameter.ts`) disappear, and `scores.{overall,java,js_ts}
.verbosity.scanned_lines` gain `+2`/`+1`/`+1` respectively (`java` from
`JavaVarargsAnnotation.java`'s surviving class wrapper, `js_ts` from
`JsxUnterminatedEntity.tsx`'s), with `verbosity.ratio` moving arithmetically
from that; `erosion` itself does not move on any of the three aggregates,
and `incomplete` stays `true` on both sides. The same capture invocation
also rewrites `malformed.report.json` unconditionally (both corpus tests
share the one `NSD_NEUTRALITY_CAPTURE` env var); that file's would-be
change was reverted byte-for-byte before committing, since it was not
authorized and this stream's own `DECLARED_DELTAS` mechanism already
tolerates the same divergence there without a baseline change — see the
round's implementer report.

### Resolved (round 3): `tests/fixtures/salvage/Mixed.java`/`Mixed.ts` joining the clean corpus

Round 2 added `CLEAN_CORPUS_ONLY_EXCLUSIONS` (`tests/neutrality.rs`),
excluding these two fixtures from `clean_corpus_sources()`'s walk instead
of following the procedure above — leaving them covered by neither Rust
corpus test, which Decision 20 does not authorize (it names this exact
addition as one that "changes clean-corpus membership … so WS-6
re-captures within its own round", the same pattern already used for the
two fixtures in the entry above). Round 3 deleted that list and let the
two fixtures join `clean_corpus_sources()` like any other fixture.

Predicted bound before capturing: each fixture contributes one clean
callable (`safe`) that is measured, and one damaged callable (`broken`)
that salvage excludes but whose surrounding wrapper lines still count per
D12 (`scanned_lines` is unconditioned on callable boundaries) —
`scores.{overall,java,js_ts}.verbosity.scanned_lines` should move by a
small positive delta, `erosion`/`ratio` should move arithmetically with
it, and nothing else (no new `skipped_files` entry: a partially damaged
file is no longer a whole-file skip; no new finding: `safe` trips none of
the six rules; no `top25` change).

**Capture**: `NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality
test_clean_corpus_report_is_byte_identical_to_the_pre_ir_baseline`
re-captured `clean.report.json` from `HEAD`; `git status --short`
afterwards showed exactly that one golden file and `tests/neutrality.rs`
itself changed (`malformed.report.json` untouched, since this narrower
invocation — unlike `scripts/neutrality_gate.sh --capture` — runs only
the one test). The resulting diff matched the prediction exactly:
`scanned_lines` moved `+10`/`+6`/`+4` (overall/java/js_ts) with
`erosion`/`ratio` moving arithmetically, nothing else. See the round 3
implementer report for the full committed diff.

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

## M0c-10: baselines moved by the Java grammar swap

The Java grammar swap (`tree-sitter-java` 0.23.5 →
`tree-sitter-java-orchard` 0.5.18, `src/parse/mod.rs`) is a deliberate
change to Java parse output, so `docs/measurements.md`'s *Known caveats*
warning about the byte-identity gate applies here too: any Java-corpus
baseline this swap moves is expected, not a regression, provided the
moved pointers are fully explained (below) rather than merely observed.

**`clean.report.json` — recaptured. Exactly four JSON pointers changed**
(`bash scripts/neutrality_gate.sh --capture`, i.e.
`NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality`, then a full
before/after pointer diff of the committed file against a
`3dd9ae2`-built control's baseline to confirm nothing else moved):

| pointer | before | after |
| --- | --- | --- |
| `/scores/java/verbosity/scanned_lines` | 636 | 637 |
| `/scores/java/verbosity/ratio` | 0.24213836477987422 | 0.24175824175824176 |
| `/scores/overall/verbosity/scanned_lines` | 1018 | 1019 |
| `/scores/overall/verbosity/ratio` | 0.22298624754420432 | 0.22276741903827282 |

All four are one mechanism: `tests/fixtures/ir/JavaVarargsAnnotation.java`
(`class JavaVarargsAnnotation { void m(Class<?> @Nullable ... cs) {} }`)
is the *only* clean-corpus file with any before/after delta at all
(confirmed by an isolated single-file scan of every non-malformed Java
fixture with both a `3dd9ae2`-built and this branch's release binary).
Under 0.23.5 its `formal_parameters` contains an `ERROR` node (the same
varargs-annotation misparse `docs/measurements.md` describes at corpus
scale), so callable `m` is entirely fail-closed excluded
(`cascade_exclusions`/`prune_damage`) and contributes nothing —
`scanned_lines` for the file is `1` (only the `class ... {` line). Under
orchard the file parses clean, `m` is fully measured, and
`scanned_lines` becomes `2` (the signature line's own leaf tokens are now
counted too, even though the empty body itself contributes `0` `sloc`).
`+1` file → `+1` java `scanned_lines` → `+1` overall `scanned_lines`; the
two `ratio` fields move only because their denominator did.

This is **not** a `modifier`/`visibility` node effect. Orchard's
node-types do add named `modifier`/`visibility` wrapper nodes around what
were previously anonymous `public`/`static`/... keyword tokens (confirmed
via a `node-types.json` diff and a real parse dump of
`tests/fixtures/rules/broken/Broken.java`), which is what this
workstream's own planning brief expected to be the delta's cause — but
that wrapper node always has exactly one child (the still-anonymous
keyword token itself), so `src/exec_lines.rs::is_executable_leaf`'s
`child_count() == 0` leaf test never accepts it, on any line, in any
fixture. The corpus-wide four-pointer diff above is the complete,
verified account of every place this swap moved `clean.report.json`; see
`docs/measurements.md`'s M0c-10 section for the same mechanism confirmed
again at the Spring/Angular perf-fixture scale, with two hand-checked
real callables.

**`malformed.report.json` — unchanged, deliberately not recaptured.**
Running the capture command touches both baseline files at once (they
share one `NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality`
invocation), and doing so once did overwrite this file too — that
recapture was reverted (`git checkout -- tests/golden/neutrality/
malformed.report.json`) after confirming the observed delta
(`/skipped_files` shrinking from three entries to zero,
`scores.{overall,java}.verbosity.scanned_lines` moving `0` → `3`) is
**already** the *Declared deltas* section's own pre-existing, WS-6-era
divergence — present in the `3dd9ae2` control build too, unrelated to and
unmoved by this swap. `git show 3dd9ae2:tests/golden/neutrality/
malformed.report.json` and the reverted file are byte-identical; a fresh
`cargo test --test neutrality` run against the reverted file passes
(`test_malformed_corpus_report_matches_its_baseline_with_declared_deltas_only`
tolerates exactly this, already-declared, delta). Recapturing it would
have replaced one already-tolerated divergence with a second,
indistinguishable one and lost the historical record of which workstream
caused which — `DECLARED_DELTAS` in `tests/neutrality.rs` is untouched by
this workstream.

## M0c-13: baselines moved by the real Top-25 span

`Callable::end_line` (M0c-13, `src/model.rs`) is now
`IrCallable::span.end_line`, populated from the AST walk, instead of a
duplicate of `start_line` — a deliberate change to what `report.json`'s
`top25[].location` publishes, so this baseline move is expected, not a
regression, provided the moved pointers are fully explained (below)
rather than merely observed.

**`clean.report.json` — recaptured.** Exactly 66 JSON pointers changed
(`NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality`, then `git diff`
grepped for every changed key name to confirm only three keys ever
appear: `end_line`, `excerpt`, `link` — 66 additions paired with 66
deletions, i.e. 22 of the corpus's 25 top-25 entries, three fields each).
The three entries that did **not** move are single-line callables whose
`end_line` already equalled `start_line` before this change: one-line
expression-bodied arrows (`<anonymous>@1` and `<anonymous>@4` in
`metrics/__tests__/NestedExpressionCallable.js`, `inner` in
`metrics/__tests__/NestedCallable.js`, e.g. `(b) => (b > 0 ? b : -b)`)
declared and closed on the line they start on, so there is nothing to
widen. Every moved entry:

| callable | file | start_line | end_line before | end_line after |
| --- | --- | --- | --- | --- |
| `decide` | `metrics/__tests__/Decisions.java` | 2 | 2 | 44 |
| `decide` | `metrics/__tests__/JsDecisions.js` | 1 | 1 | 39 |
| `compute` | `metrics/erosion/HighComplexity.java` | 2 | 2 | 12 |
| `classify` | `report_erosion/src/HighComplexity.java` | 2 | 2 | 16 |
| `m` | `ir/Decisions.java` | 2 | 2 | 28 |
| `m` | `ir/decisions.ts` | 1 | 1 | 27 |
| `lowComplexity` | `metrics/erosion/LowComplexity.js` | 1 | 1 | 11 |
| `loopy` | `metrics/__tests__/BareControlFlow.js` | 1 | 1 | 11 |
| `run` | `clones/__tests__/SwitchDupJava.java` | 2 | 2 | 29 |
| `run` | `clones/__tests__/SwitchDupJs.js` | 1 | 1 | 37 |
| `forOfOptional` | `metrics/__tests__/ForOfOptional.js` | 1 | 1 | 9 |
| `colonForm` | `metrics/__tests__/SwitchForms.java` | 2 | 2 | 12 |
| `arrowForm` | `metrics/__tests__/SwitchForms.java` | 14 | 14 | 21 |
| `m` | `ir/ContainerSet.java` | 7 | 7 | 13 |
| `m` | `ir/StructuralPredicates.java` | 2 | 2 | 10 |
| `m` | `ir/container_set.ts` | 4 | 4 | 10 |
| `m` | `ir/structural_predicates.ts` | 1 | 1 | 9 |
| `Point` | `metrics/__tests__/CallableKinds.java` | 10 | 10 | 14 |
| `outer` | `metrics/__tests__/NestedCallable.js` | 1 | 1 | 7 |
| `sloc` | `metrics/__tests__/SlocLines.js` | 1 | 1 | 12 |
| `ifConstruct` | `parity/Constructs.java` | 2 | 2 | 8 |
| `forConstruct` | `parity/Constructs.java` | 10 | 10 | 14 |

Each row's `excerpt` widened to match (the callable's whole declaration
and body, read back off disk, not just its first line) and each row's
`link` moved from `#L{n}-L{n}` to `#L{start}-L{end}` accordingly — no
other field on any row, and no field outside `top25[]`, moved.

**`malformed.report.json` — unchanged.** Its own `top25` array is empty
(the corpus is `incomplete`, fail-closed on every fixture's one
callable), so the span widening has nothing to reach there; a capture
run touched it anyway (recapturing both files at once), and that
recapture was reverted (`git show HEAD:tests/golden/neutrality/
malformed.report.json > tests/golden/neutrality/malformed.report.json`,
`git checkout --`/`git restore` being off-limits) after confirming the
only observed delta — `/skipped_files` shrinking from three entries to
zero, `scores.{overall,java}.verbosity.scanned_lines` moving `0` → `3`
— is the *Declared deltas* section's own pre-existing, WS-6-era
divergence, unrelated to and unmoved by this workstream: a fresh
`cargo test --test neutrality` run against the reverted file passes
(`test_malformed_corpus_report_matches_its_baseline_with_declared_deltas_only`
tolerates exactly this, already-declared, delta).

## The `java-fixture-01` strict leg (retired at M0c-10)

Spec item 8 named a byte-identity comparison against `java-fixture-01`,
the private archived report — separate from `docs/golden-digest.md`'s
lossy digest comparison (below). Through the end of M0b,
`tests/neutrality.rs::test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report`
covered it, gated the same way `tests/golden_digest.rs` gates its own
archive comparison: `NSD_ARCHIVED_REPORT` (a path to the private archived
`report.json`, read only at test run time) resolved the leg; when unset,
the test printed a pending notice and passed, unless
`NSD_REQUIRE_ARCHIVE_VERIFIED` was also set, in which case it panicked.
`test_java_fixture_01_strict_leg_is_reported_pending_when_the_archive_is_absent`
pinned the pending path. When resolved, the archived report's own
`scan.{target,include_tests,exclude,min_clone_lines}` were read back out of
it to rebuild the exact `ScanSettings` used to produce it, a fresh scan was
run, and the two `report.json` texts were compared byte-for-byte.

**Last run before retirement (run by the coordinator on `main@3dd9ae2`,
still at `tree-sitter-java` 0.23.5): PASS.** `NSD_ARCHIVED_REPORT=<the
private archive> NSD_REQUIRE_ARCHIVE_VERIFIED=1 cargo test --test
neutrality` ran
`test_java_fixture_01_strict_scan_is_byte_identical_to_the_archived_report`,
which scanned `java-fixture-01` byte-identical to the archived
`report.json` — 2,613,042 bytes, verified against a tampered-archive
negative control (a single flipped byte in a scratch copy of the archive
correctly failed the comparison). See `docs/implementation-status.md`,
row M0b-8c, for the commit that records
this without the archive's path or contents.

**Retired by this workstream (M0c-10), not merely re-baselined.** Item 8's
invariant was "the IR retarget alone produces a byte-identical
`report.json`" — a statement about IR *lowering*, holding the *parser*
fixed. M0c-10 deliberately changes the parser (`tree-sitter-java` 0.23.5 →
`tree-sitter-java-orchard` 0.5.18) specifically so it no longer misparses a
type-use annotation before a varargs ellipsis. Re-pointing this leg at a
newly-captured `java-fixture-01` archive would silently convert a
parser-behavior invariant into "whatever today's grammar happens to
produce", which is not what item 8 asked for and not verifiable without
re-running the private fixture archival process (out of this workstream's
scope; WS-5 owns the post-swap golden digest instead, in
`docs/golden-digest.md`). The test and its pending twin have been removed
from `tests/neutrality.rs` rather than left to assert against a stale
pre-swap archive forever. The `java-fixture-01` corpus's *lossy* digest
comparison (`tests/golden_digest.rs`, `docs/golden-digest.md`) is
unaffected by this retirement and continues to gate independently.

## Running the gate

```
cargo test --test neutrality
```

runs the always-on comparison against the committed baselines; it needs
nothing beyond the checked-out repository. (Through M0b this also carried
the optional, private-archive-gated `java-fixture-01` strict leg described
above; that leg is retired as of M0c-10 and no longer part of this
suite.)

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

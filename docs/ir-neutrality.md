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

0. **Step 0, a precondition on every step below:** `git status --short
   tests/fixtures` must print nothing. Step 3's `--capture` reads whatever
   is on disk under `tests/fixtures/` at the moment it runs, committed or
   not, and writes it straight into the golden file with no review step of
   its own — an unrelated, uncommitted fixture edit sitting alongside the
   one this procedure means to capture would be baked in silently,
   indistinguishable afterwards from the intended change.
1. **Proof A**, ground truth: `bash scripts/neutrality_gate.sh` with no
   arguments. It builds the pinned pre-IR commit in a detached worktree and
   the current `HEAD`, and diffs their `report.json` directly against each
   other on the same copied corpus, so it is immune to corpus drift by
   construction. It must print `NEUTRALITY: identical on 2 corpora`; if it
   reports a divergence, stop — a capture on a diverged tree is a silent
   re-baseline, not an explained one. **As of M0c, this step no longer
   gates** (B16, task.md): `PRE_IR_SHA` (`scripts/neutrality_gate.sh:43`,
   `5705e76522b7b0a16343cd861aa953e6ff37064a`) is the last commit before
   any M0b IR-retargeting stream touched an analyzer, long before the M0c
   Java grammar swap (`tree-sitter-java` 0.23.5 → `tree-sitter-java-orchard`
   0.5.18, see *M0c-10* below) moved four `clean.report.json` pointers in a
   deliberate, fully-explained way. Proof A diffs `HEAD` directly against
   that pinned commit's own live render with no `DECLARED_DELTAS`-style
   escape hatch, so it is red by construction on every run made after M0c
   regardless of corpus drift — measured on this stream's own `HEAD`,
   `bash scripts/neutrality_gate.sh` prints `NEUTRALITY: clean corpus
   diverged at /scores/java/erosion` — the first pointer jq reports
   (sorted keys) comes from salvage now measuring five previously
   skipped files; the same run also carries M0c-10's four verbosity
   pointers and M0c-13's `/top25/*/location` spans, 76 pointers in all,
   none of them a corpus-membership bug, and not something re-pointing
   `PRE_IR_SHA` forward would honestly fix (see the trust-anchor
   paragraph below).
   Treat Proof B as this procedure's actual corpus-drift check; Proof A
   stays meaningful only run against a pre-M0c commit, or once a human
   deliberately re-pins `PRE_IR_SHA` to a post-M0c commit with its own
   fresh proof that *that* commit is neutral against its predecessor.
2. **Proof B**, a drift bound: a control scan of the clean corpus minus the
   changed/added fixtures, compared byte-for-byte against the *currently
   committed* `clean.report.json`. This proves no pre-existing fixture's
   numbers moved, so the only thing the re-capture can introduce is the
   new file(s)' presence. **Fixture deletion is the mirror case:** scan
   the clean corpus *as currently committed* — still including the
   file(s) about to be deleted — against the same currently-committed
   `clean.report.json` first; it must be byte-identical, since nothing has
   changed yet. Delete the fixture(s), then proceed to step 3; step 4's
   diff is then bounded to exactly the deleted file's own contribution
   disappearing (its `/scores/*` share and any `/top25` row it held), the
   same shape as an addition, signed the other way.
3. Run `bash scripts/neutrality_gate.sh --capture` to rewrite the golden
   files from `HEAD`'s own render — never hand-edit or reconstruct the JSON.
4. Check the resulting diff is bounded to exactly what corpus growth
   predicts: the affected `/scores/*` aggregates (denominators and any
   ratios/erosion derived from them) and, if the new file's callables rank
   into the top 25, the displaced `/top25` rows — nothing else.

**`PRE_IR_SHA` (`scripts/neutrality_gate.sh:43`) is this gate's trust
anchor and is never re-pointed to force Proof A green.** It fixes what
"pre-IR" means for every re-capture that follows; moving it forward to
make a present divergence disappear would let a later, unrelated
measurement regression hide behind whatever the new pin's own behavior
happens to be — the same failure mode `AGENTS.md` warns against for
`--staged` and trusted policy ("candidate configuration cannot weaken its
own gate"). A divergence Proof A reports is either fully explained (as
M0c-10 is, both here and in `docs/measurements.md`) or it is a stop
signal — never silently absorbed by moving the pin.

`tests/fixtures/salvage/` (WS-6) was the fixture addition this note
originally anticipated triggering this procedure, and it did: `Mixed.java`
and `Mixed.ts` join the clean corpus (see the *Resolved (round 3)* entry
below) exactly as Decision 20 (`plan.md:1000`, `:1030`) prescribes — an
earlier round of this stream instead excluded them from the clean-corpus
walk via a `CLEAN_CORPUS_ONLY_EXCLUSIONS` list, which has since been
deleted. WS-6's own brief separately forbids regenerating either baseline
under `tests/golden/neutrality/` at all (`AGENTS.md`, *Verification*:
"never substitute invented results or silently re-baseline an unexplained
difference"), which this procedure remains available for a human operator
to run deliberately, with the two proofs above, if a *genuine*
corpus-composition change is ever needed.

### Resolved: two pre-existing damaged fixtures under `tests/fixtures/ir/`

WS-6 found that `tests/fixtures/ir/JavaVarargsAnnotation.java`, `JsxUnterminatedEntity.tsx` and
`TsUsingParameter.ts` carry a `SyntaxError` yet belong to the clean corpus; salvage now measures
them partially, and `clean.report.json` was recaptured through the procedure above (their
`skipped_files` entries gone, `scanned_lines` up `+2`/`+1`/`+1` overall/java/js_ts, `erosion` unchanged).

### Resolved (round 3): `tests/fixtures/salvage/Mixed.java`/`Mixed.ts` joining the clean corpus

Both fixtures joined `clean_corpus_sources()` through the procedure above (the interim
`CLEAN_CORPUS_ONLY_EXCLUSIONS` list was deleted); the recapture moved only `scanned_lines`
(`+10`/`+6`/`+4` overall/java/js_ts) and, arithmetically, `erosion` and `ratio`.

### Resolved (WS-7): `tests/fixtures/erosion_boundary/Boundary.java` joining the clean corpus

`Boundary.java` (one callable at exactly `cc == 10`, pinning the `cc > 10` erosion boundary
against a `>=` mutation) joined the clean corpus through the procedure above; the recapture moved
only `scanned_lines` (`+13` overall/java), `ratio`, `erosion` and `top25`, and
`malformed.report.json` stayed byte-identical.

## Normalization

None is left (M6-1/M6-2, WS-3). Before, the corpus's own temporary-directory
path was replaced with the label `<neutrality-corpus>` in the raw
`report.json` text (`normalize_raw_text`) and at the `Value` level
(`normalize`), because `scan.target` held it. `scan.target` is now `null` and
the report carries no excerpt, so the report names the path nowhere:
`tests/neutrality.rs::path_free_raw_text` asserts **zero** occurrences of it
and the byte-identity comparison reads the raw text `nsd` wrote as is (struct
declaration order, exact whitespace), not a re-serialized `serde_json::Value`
(whose maps use alphabetical key order and would not detect a real
serialization regression).
`test_scan_report_names_no_corpus_path_and_has_a_null_target` pins this.

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
against both a changed leaf and a key present on only one side. It is empty
since M1-8, which folded the moves below into the recaptured
`malformed.report.json` (see *M1-8: baselines moved by completeness and gap
provenance*), so any difference against that baseline is a regression. WS-6
had populated it with exactly three pointers, one by one:

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

## M1-8: baselines moved by completeness and gap provenance

M1-8 changes the format of `report.json` on purpose, so byte-identity Proof B
cannot pass across it. The adapted proof, run **before** recapturing, is
`tests/neutrality.rs::test_previous_baselines_differ_only_by_enumerated_format_deltas`.
It compares the still-committed baseline of each corpus with the live scan
after `strip_enumerated_format_deltas` removed, from both sides and only when
present: `unanalyzed_lines` and `complete` under each
`scores.{overall,java,js_ts}.verbosity`, and `gaps` and `unmeasured_callables`
from each `parse_syntax_error` `skipped_files` row. No other key or subtree is
normalized, and the malformed corpus still passed its then-declared WS-6
deltas. It passed on the old baselines, so M1-8 moved no measurement
(`cc`, `sloc`, `mass`, `scanned_lines`, `flagged_lines`, `ratio`, `erosion`,
findings, duplicates, `top25`) on either corpus. The malformed comparison
could not see the pointers its then-declared WS-6 deltas masked
(`/skipped_files` and `/scores/{overall,java}/verbosity/scanned_lines`);
those were checked separately by the recapture diff listed below.
`test_format_delta_comparator_catches_a_planted_measurement_mutation` proves
the comparator reports a planted `top25/0/cc`, `top25/0/sloc`,
`verbosity.scanned_lines`, `verbosity.ratio` and `erosion` change at exactly
its own pointer, and ignores a change to an enumerated key.

The baselines were then recaptured with `NSD_NEUTRALITY_CAPTURE_CLEAN=1` and
`NSD_NEUTRALITY_CAPTURE_MALFORMED=1` (one test each; `PRE_IR_SHA` untouched).

**`clean.report.json`** moved at these pointers only:

- `/scores/{overall,java,js_ts}/verbosity/unanalyzed_lines` and `/complete`
  added (5/2/3 unanalyzed lines, all `complete: false`): the clean corpus holds
  four salvaged fixtures (`ir/JsxUnterminatedEntity.tsx`, `ir/TsUsingParameter.ts`,
  `salvage/Mixed.java`, `salvage/Mixed.ts`).
- `/skipped_files/<i>/gaps` and `/unmeasured_callables` added on those four rows.
- No existing value moved.

**`malformed.report.json`** moved at these pointers:

- The same added `unanalyzed_lines`, `complete`, `gaps` and `unmeasured_callables`
  keys (WS-1).
- `/scores/overall/verbosity/scanned_lines` and
  `/scores/java/verbosity/scanned_lines` `0` -> `3`, and each
  `/skipped_files/<i>/detail` `null` -> `salvaged; first error at line N`: these
  are the WS-6-era moves that `DECLARED_DELTAS` had been tolerating against a
  baseline never recaptured. M1-8 did not cause them; the recapture absorbs them.

## M6-1/M6-2: baselines moved by the canonical scan report

`report.json` became the canonical scan document on purpose (WS-3), so
byte-identity Proof B cannot pass across it. The adapted proof ran **before**
recapturing: `tests/neutrality.rs::test_previous_baselines_differ_only_by_enumerated_format_deltas`
compared the still-committed baselines with the live scans after stripping the
M1-8 deltas and these WS-3 ones from both sides, and passed with no other
pointer moved on either corpus, so no measurement moved.

The enumerated WS-3 deltas (`ADDED_TOP_LEVEL_KEYS`, `strip_excerpts` and the
`scan.target` removal in `strip_enumerated_format_deltas`):

- Added top-level keys: `/schema_version`, `/result_scope`, `/snapshots`,
  `/fingerprints`, `/skipped`, `/callables`.
- `/scan/target`: the label `<neutrality-corpus>` became `null`.
- Every `excerpt` key (`/findings/*/location`, `/duplicates/*/locations/*`,
  `/top25/*/location`) is gone from the JSON; the HTML keeps them.

`test_format_delta_comparator_catches_a_planted_measurement_mutation` also
plants each of these on a live report and asserts none is reported, while a
neighbour of `scan.target` (`/scan/include_tests`) and a moved `cc` still are.
Both baselines were then recaptured with
`NSD_NEUTRALITY_CAPTURE_CLEAN=1 NSD_NEUTRALITY_CAPTURE_MALFORMED=1 cargo test
--locked --test neutrality` after `git status --short tests/fixtures` printed
nothing (step 0), and `cargo test --locked --test neutrality` passes without
the variables. `PRE_IR_SHA` and `scripts/neutrality_gate.sh` are untouched.
The malformed baseline's `callables` is `[]` (every fixture's one callable is
damaged); `DECLARED_DELTAS` stays empty.

## M7-1: baselines moved by the `nested_checkout` skip key

`scan`'s `skipped` object gained an 11th key, `nested_checkout` (sorted between
`gitignore` and `parse_grammar_setup`), so every scan report carries
`"nested_checkout": 0` for a corpus with no nested checkout. WS-3 also added
the WebJar glob and the fixture, `e2e` and QA test globs; none matches a path
below `tests/fixtures/` as the harness copies it (no `fixture`, `fixtures`,
`__fixtures__`, `e2e`, `qa` or `webjars` component, no `.e2e-spec.` name), so
no file moved between discovered and skipped.

The semantic Proof B ran **before** recapturing:
`tests/neutrality.rs::test_previous_baselines_differ_only_by_enumerated_format_deltas`
passed against the old baselines, and the byte-identity tests failed only on
the one added pointer `/skipped/nested_checkout`. Both baselines were then
recaptured with `NSD_NEUTRALITY_CAPTURE_CLEAN=1 NSD_NEUTRALITY_CAPTURE_MALFORMED=1
cargo test --locked --test neutrality`; `git diff` of `tests/golden/neutrality/`
shows exactly one added line per file, `"nested_checkout": 0,`, and
`cargo test --locked --test neutrality` passes without the variables. No
measurement moved.

## M0c-10: baselines moved by the Java grammar swap

The Java grammar swap (`tree-sitter-java` 0.23.5 →
`tree-sitter-java-orchard` 0.5.18, `src/parse/mod.rs`) is a deliberate
change to Java parse output, so `docs/measurements.md`'s *Known caveats*
warning about the byte-identity gate applies here too: any Java-corpus
baseline this swap moves is expected, not a regression, provided the
moved pointers are fully explained (below) rather than merely observed.

**`clean.report.json` — recaptured. Exactly four JSON pointers changed**
(`bash scripts/neutrality_gate.sh --capture`, i.e.
`NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality` [Superseded: the legacy `NSD_NEUTRALITY_CAPTURE` is now `NSD_NEUTRALITY_CAPTURE_CLEAN` / `NSD_NEUTRALITY_CAPTURE_MALFORMED`; see "Running the gate".], then a full
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
share one `NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality` [Superseded: the legacy `NSD_NEUTRALITY_CAPTURE` is now `NSD_NEUTRALITY_CAPTURE_CLEAN` / `NSD_NEUTRALITY_CAPTURE_MALFORMED`; see "Running the gate".]
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
(`NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality` [Superseded: the legacy `NSD_NEUTRALITY_CAPTURE` is now `NSD_NEUTRALITY_CAPTURE_CLEAN` / `NSD_NEUTRALITY_CAPTURE_MALFORMED`; see "Running the gate".], then `git diff`
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

## PV-1: baselines moved by the Rust lowering

Adding `tree-sitter-rust` and the third lowering moved both baselines in
exactly two places, and nothing else:

1. `fingerprints.measurement` became `blake3:ee1b3117a8eca80df2994125c61fe745`
   (it was `blake3:43340e59d6e269babeb275808dda9960`): the two new hashed
   inputs, `rust_lowering_version` and `rust_grammar_version`
   (`docs/measurement-profile.md`).
2. `scores.rust`, a zeroed family row (`erosion` 0.0; verbosity
   `complete: true`, every count 0), because neither corpus holds a `.rs`
   file.

No `.rs` file was added under `tests/fixtures/`, so no callable, clone group,
finding or line count moved. Both baselines were recaptured with
`scripts/neutrality_gate.sh --capture`; `git diff` of
`tests/golden/neutrality/` shows only the two changes above in each file, and
`tests/golden/*.digest.json` is unchanged. `cargo test --locked --test
neutrality --test golden_digest` passes without the capture variables.

## The `java-fixture-01` strict leg (retired at M0c-10)

Through M0b, `tests/neutrality.rs` carried a private-archive-gated
byte-identity leg against `java-fixture-01` (last run PASS on `main@3dd9ae2`,
status row M0b-8c); M0c-10 retired it because it pinned the pre-swap parser's
behaviour, and the lossy digest comparison (`tests/golden_digest.rs`,
`docs/golden-digest.md`) gates `java-fixture-01` instead.

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
`cargo test --test neutrality` with `NSD_NEUTRALITY_CAPTURE_CLEAN=1` and
`NSD_NEUTRALITY_CAPTURE_MALFORMED=1` set — the same raw-text capture path
the Rust harness's two corpus tests use, so the committed baseline is
guaranteed byte-identical to what the compare mode would later read. It
does not touch the pre-IR worktree at all.

Each corpus test is gated by its own variable, so recapturing one corpus
directly (`NSD_NEUTRALITY_CAPTURE_CLEAN=1 cargo test --test neutrality`)
never rewrites the other's baseline. The older shared
`NSD_NEUTRALITY_CAPTURE` no longer selects anything; earlier sections of
this document that name it describe how those captures were run at the
time.

## What this gate is not

Beyond the `java-fixture-01` strict byte-identity leg described above,
this gate does not cover the wider ten-suite private archive comparison or
`docs/measurements.md`'s perf fixture — those stay outside this repository
entirely (`AGENTS.md`, *Fixture privacy*) and are covered by
`docs/golden-digest.md` and `tests/golden_digest.rs`'s lossy digest
comparison instead. No private repository name, checkout path, source
excerpt, or name mapping appears anywhere in this document or in
`tests/golden/neutrality/`.

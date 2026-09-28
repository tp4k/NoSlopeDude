# Scan time and peak memory (D24/D25)

`## Verification`'s own words: "Record scan time and peak memory on a
larger fixture, using roughly **1 million source lines as a target rather
than a release gate**." Nothing in this file, and no CI job anywhere in
this repo, turns that target into an assertion on seconds or megabytes —
the rows below are informational.

## The fixture (D25)

The fixture is two cloned public repositories, not a generated tree
(Q3, answer b):

- `https://github.com/spring-projects/spring-framework`, pinned at
  `e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9` — the Java half.
- `https://github.com/angular/angular`, pinned at
  `a783c4e7b753929ababa610e305112b82aaa0eb0` — the JS/TS half.

`scripts/fetch_perf_fixture.sh` shallow-fetches both into one fixture root
(default `${TMPDIR:-/tmp}/nsd-perf-fixture`, overridable by its
first argument), pinned by sha with a same-URL default-branch fallback if
a pin is ever unreachable. `scripts/perf_scan.sh` then runs exactly one
scan of that root under `/usr/bin/time -l` and appends a row below.

The fixture root lives **outside this repo's tree** (not a gitignored
in-repo directory): the scanner's own walk honours a parent `.gitignore`
by default (`require_git(false)`, `WalkBuilder::parents`), so an ignored
in-repo path would be invisible to the scan itself and the measurement
would silently run against an empty tree. Neither script writes anything
under this repository.

## Accepted tradeoffs

- **Network-dependent.** Both scripts need network access to GitHub and
  must stay out of `cargo test`; `perf_scan.sh` refuses to run without an
  already-fetched fixture rather than measuring an empty tree.
- **Not comparable across pins.** The fixture's size is upstream's to
  change — re-pinning either repository to a newer sha changes the
  scanned-source-line count, so rows taken at different pins move for
  reasons unrelated to the scanner. This is why every row below carries
  the shas actually scanned and its own scanned-source-line count (read
  from that run's own `report.json`), rather than being compared on wall-
  clock seconds alone.

## Known caveats of the recorded rows (round 2)

This section originally described every row below as coming from one
uniform incomplete scan: 58 whole files (55 Java, 3 JS/TS) dropped
entirely from every score. That was accurate only through the
`2026-09-23` rows, including the three `a370e3d` controls. It is stale
for every row from the `M0c-10` orchard row onward (2026-09-24 and later,
WS-8, re-measured against the pinned fixture): those rows still carry
`incomplete: true`, but for a narrower and no-longer-whole-file reason.

**55 of the 58 parse failures are gone.** The `tree-sitter-java` 0.23.5
→ `tree-sitter-java-orchard` 0.5.18 swap (`## M0c-10` below) clears every
Java `SyntaxError` (the `Class<?> @Nullable ... cs` varargs-annotation
misparse) to zero; 3 `tree-sitter-typescript` 0.23.2 `SyntaxError`
failures on a parameter literally named `using` remain, confirmed
unrelated to and unchanged by the swap.

**None of the remaining 3 are whole-file drops any more.** A separate,
already-landed change (WS-6's salvage) means a `SyntaxError` file keeps
its `ParsedFile`: only the callable(s) or block(s) whose own subtree
touches damage anywhere, and anything nested inside one of them, are
fail-closed excluded (`src/lower/mod.rs`'s `cascade_exclusions`: an
entity is excluded if it is itself dirty or its nearest enclosing entity
is excluded), not the file's every other callable. `report.json`'s
`incomplete: true` still holds on every row here regardless of this
change — `parse_failures` is deliberately kept non-empty for exactly
that flag's sake — so `incomplete` is no longer evidence of a whole-file
drop by itself. The row's `skipped files` cell is `report.json`'s full
`skipped_files` length; the same salvage change filters every
`SyntaxError` entry out of that list too, so for every row from `M0c-10`
onward it is discovery-time exclusions only (test directories, generated
code, dependency/build output under D16's default rules) — the 55 Java +
3 JS/TS parse failures contribute to it not at all, cleared or not. The
seven `2026-09-18`/`2026-09-23` rows still read 7720 (`7662 + 58`)
because they predate this change. The `3dd9ae2` control row (`## M0c-10`
below) already reads 7662, not 7720, despite predating the grammar swap
in its own binary: it was built one commit before the swap but after
WS-6's salvage had already landed on this branch, so its 55 Java + 3 TS
`SyntaxError`s are already excluded from `skipped_files` — none of them
whole-file — and only the swap's own Java-parse-failure clearance
(0 vs 55) was still pending in that binary.

Re-measured directly against the pinned fixture with today's
`scripts/perf_scan.sh` (the `nsd@403aa7f7…` row) reproduces the `M0c-10`
row's own `scanned_lines` (584779) and `skipped files` (7662) exactly,
corroborating that this is the scan's current, standing behaviour rather
than a one-off measurement.

Every row not marked `block input operations not recorded` was taken with a
**warm page cache** (`/usr/bin/time -l`'s own `block input operations: 0`),
so wall-clock seconds do not include first-touch disk I/O. Across these two
fixtures, peak RSS scales roughly
linearly with total source bytes at **≈27×** (the pipeline retains every
parsed tree and source file simultaneously rather than streaming
file-by-file), so extrapolating to a much larger corpus should scale the
RSS estimate by source-byte count, not by scanned-line count alone.

## Provenance of the 2026-09-18 rows

The two `2026-09-18` rows (below) entered this repository with `37829cb`,
the commit that imported the `agent_slope` engine at upstream sha
`912ec7a`: they are upstream's own pre-import numbers, measured before
this crate was even named `nsd` and before any of the M0b IR-retargeting
branch's commits existed. So the 2026-09-18 → 2026-09-23 delta in the
table below spans the *entire* M0b IR branch — the IR's introduction, its
corpus-wide retention in `PipelineOutput::ir`, every stage's retarget, and
this stream's own two-lowering fusion (`e83228b`) among roughly forty
other commits — and **is not attributable to any single one of them**,
this stream's fusion included. The arithmetic confirms it independently:
after the fusion the corpus is lowered 4 times per scan where it was
lowered 5 times, so even if lowering were 100% of wall clock, the ceiling
on the fusion's own possible saving is 20% (17.54s → at best ~14.0s) — the
observed 76% drop (17.54s → 4.28s) is arithmetically unreachable from
removing one of five redundant lowerings alone, and is therefore evidence
about the branch, not about this fix.

The fusion's own effect was, for that reason, unmeasured here for want of
a control. Round 4 supplied a first attempt: the two unlabeled
`2026-09-23` rows in `## Rows` below measure `e83228b` (the fusion commit) via two
back-to-back `scripts/perf_scan.sh` runs at 18:53. About 20 minutes
later, in a separate detached worktree (the `git worktree add --detach`
pattern `scripts/neutrality_gate.sh` uses for its own pre-IR reference
build, but built with `cargo build --release` — the profile
`perf_scan.sh` itself uses, **not** `neutrality_gate.sh`'s own
`cargo build --quiet` debug profile), a hand-replicated equivalent of
that script measured `a370e3d` (the fusion's immediate parent) once,
immediately after that worktree's from-scratch release build. That
single run was **not** a back-to-back, like-for-like comparison with the
`e83228b` pair: n=1 vs. n=2 (4.28s/4.14s), a cold build immediately
preceding it rather than the warm steady state the `e83228b` rows were
taken in, and — unlike every other row in this file — without a recorded
`block input operations` value, so `## Known caveats`'s universal
warm-page-cache claim was, for that one row, asserted rather than
verified. The round-4 report read the resulting drop (6.90s → 4.28s/4.14s,
~39%) as evidence the pair "isolates the fusion's own effect" — that
claim was wrong on its own terms: this section's own arithmetic above
caps the fusion's possible saving at ≤20% (≤1.38s off a 6.90s parent),
and 2.69s is ~1.95× that ceiling, so at least ~1.3s of that recorded
delta cannot be the fusion's — most plausibly session-to-session variance
(cold build/cache), not evidence for or against `e83228b`.

Round 5 re-ran the control twice more, same worktree pattern and release
build, same already-fetched fixture root, this time recording
`block input operations` for each run and treating the original 6.90s run
as the cold-build warm-up it evidently was (its row below is relabeled
accordingly; its number is untouched): 6.77s (1778.98 MB,
`block input operations: 0`) and 5.68s (1777.41 MB,
`block input operations: 0`) — both warm, both n=1 individually, mean
6.225s. Against the `e83228b` pair's mean of 4.21s that is still a ~32%
drop, and even the closer rerun alone (5.68s vs. 4.21s) is ~26% — both
above the ≤20% ceiling this section's own arithmetic derives from the
5→4 per-scan lowering count. **That means the ceiling arithmetic itself
needs re-examining, not that a ~32% drop is unreachable**: either the
lowering count alone does not capture the fusion's full effect on wall
clock (e.g. avoided allocation/copy overhead beyond the lowering call
itself), or some other uncontrolled difference between the two worktrees
(filesystem cache locality, thermal state, background load) still
separates them despite the warm rebuild. Three `a370e3d` runs (6.90s,
6.77s, 5.68s) against two `e83228b` runs (4.28s, 4.14s) is still not the
same reproducibility bar on both sides (n=3 vs. n=2, taken across two
sessions ~20 minutes to hours apart, never back-to-back). The within-side
spread alone is not small: the three control runs span 1.22s (6.90s −
5.68s, ~18% of the 6.90s parent), and the two warm reruns alone span 1.09s
(6.77s − 5.68s, ~18% of their 6.225s mean) — a share of the "still above
ceiling" gap that could be ordinary run-to-run noise on the control side,
not evidence the ceiling model itself is wrong. This file records what was
measured, not a settled attribution of the fusion's own effect.

## M0c-10: the Java grammar swap's own measured effect

The two `2026-09-24` (`M0c-10`) rows in `## Rows` below scan the same already-fetched fixture root, at
the same pins, one after the other in the same session: the control row was
built with `cargo build --release` in a `git clone` of this worktree
checked out to `3dd9ae2` (this branch's own base, one commit before the
`Cargo.toml` edit) in a directory entirely outside this repository's tree,
so the swap commit never touched the binary that produced it; the second
row is this branch's own release build. Neither `skipped_files` count
(7662, unchanged) reflects the parse-failure clearance below —
`src/report/mod.rs::build_skipped_files` already filters
`ParseFailureReason::SyntaxError` out of that list (a WS-6 salvage-era
change: a syntax-error file is no longer a whole-file skip, only its
touched callables are fail-closed excluded), so `report.json`'s
`skipped_files` cannot answer "did the gate clear" either way — this is
exactly why `tests/grammar_gate.rs` reads `PipelineOutput::parse_failures`
directly instead. (The two much older `2026-09-18`/`2026-09-23` rows in `## Rows` below
read `7720`, i.e. `7662 + 58`, at this same pin because they predate the
WS-6 change entirely: back then the 55 Java + 3 TS syntax-error files were
still whole-file skips. That -58 is WS-6's own delta, already explained by
`tests/neutrality.rs::DECLARED_DELTAS`'s comment on the malformed corpus,
and unrelated to this swap.)

**Parse-failure count** (`tests/grammar_gate.rs`, `NSD_PERF_FIXTURE` run,
and independently cross-checked with a throwaway `cargo run --example`
probe built against the `3dd9ae2` control clone reading the same
`PipelineOutput::parse_failures` field): 55 Java `SyntaxError` failures
under 0.23.5, all a `tree-sitter-java` misparse of a type-use annotation
immediately before a varargs ellipsis (`Class<?> @Nullable ... cs` —
`@Nullable`'s `.` lookalike inside `...` gets consumed as a
`scoped_identifier`, producing an `ERROR` node under `formal_parameters`);
**0 under orchard 0.5.18** — the gate clears. **3 TS `SyntaxError` failures
remain unchanged** (a `tree-sitter-typescript` gap on a parameter literally
named `using`; JS/TS grammars are untouched by this swap, confirmed below).

**Corpus-wide score delta** (full-fixture scan, both binaries, this
session; `overall`/`java`/`js_ts` blocks of `report.json`):

| metric | before (0.23.5) | after (orchard) | delta |
| --- | --- | --- | --- |
| java `verbosity.scanned_lines` | 289,447 | 289,877 | +430 |
| java `verbosity.flagged_lines` | 6,901 | 6,906 | +5 |
| java `erosion` | 0.36215223499124904 | 0.3619381396927912 | -0.00021 |
| findings total | 3,171 | 3,173 | +2 |
| `JAVA-REDUNDANT-ELSE-AFTER-RETURN` | 2,062 | 2,064 | +2 |
| `JAVA-EMPTY-CATCH` | 60 | 60 | 0 |
| duplicate groups | 258 | 258 | 0 |
| duplicate `redundant_lines` (sum across all 258 groups) | 6,911 | 6,911 | 0 |
| js\_ts `verbosity.scanned_lines` | 294,902 | 294,902 | 0 |
| js\_ts `erosion` | 0.45137589185379046 | 0.45137589185379046 | 0 |

The `js_ts` row being bit-for-bit identical on both sides is itself a
neutrality proof: this swap changes nothing observable for JS/TS, as
required (JS/TS grammar deps are untouched in `Cargo.toml`).

**What the +430/+5/+2 java deltas are attributable to.** Not, contrary to
this workstream's own planning brief, `modifier`/`visibility` node
wrapping: a direct AST dump (`tree-sitter-java-orchard` 0.5.18, node-types
and a real parse of `tests/fixtures/rules/broken/Broken.java`) shows
`modifiers`'s former anonymous keyword children (`public`, `static`, ...)
are now wrapped in a named `modifier`/`visibility` node, but that wrapper
node always has exactly one child (the still-anonymous keyword token
itself) — so `src/exec_lines.rs::is_executable_leaf`'s `child_count() == 0`
leaf test never accepts it, on any line, in any file. Confirmed
empirically, not just structurally: a corpus-wide before/after diff of
every local golden baseline (`tests/golden/neutrality/{clean,malformed}
.report.json`) and a per-file isolated-scan sweep of every non-malformed
Java fixture found exactly one file with any scanned-line delta at all
(`tests/fixtures/ir/JavaVarargsAnnotation.java`, `+1`), and zero files
where a `modifier`/`visibility` node changed anything.

The actual mechanism, on both the local fixture and the real Spring files
above, is M0c-10's own headline effect: **fail-closed damage exclusion
lifting**. Before the swap, any callable whose `formal_parameters`
contained the varargs-annotation `ERROR` node was entirely excluded from
measurement (`src/lower/mod.rs`'s `cascade_exclusions`/`prune_damage`,
fail-closed at the callable boundary) — contributing zero `sloc`/`cc`/
`mass`/scanned-lines/findings, no matter how large its real body. After the
swap the same callable parses clean and is fully measured. Three real
callables, hand-checked (paths below are the pinned public fixture's own,
not privacy-sensitive): `getMethodIfAvailable` and `findMethod` by copying
each verbatim into an isolated single-file scan (scratch fixtures scanned
with both release binaries); `readCode`'s `cc`/`sloc`/`mass` read from the
`top25` entry of both binaries' full-fixture `report.json` instead, since
it was already measured pre-swap and needs no isolation to observe:

| callable | before (0.23.5) | after (orchard) |
| --- | --- | --- |
| `org.springframework.util.ClassUtils#getMethodIfAvailable` (`Class<?> clazz, String methodName, @Nullable Class<?> @Nullable ... paramTypes`) | excluded: file `incomplete=true`, callable absent from every measurement (`sloc`/`cc`/`mass` = 0/unmeasured) | `cc=3`, `sloc=8`, `mass=8.485281374238571` |
| `org.springframework.util.ReflectionUtils#findMethod` (`Class<?> clazz, String name, Class<?> @Nullable ... paramTypes`) | excluded: file `incomplete=true`, callable absent from every measurement | `cc=7`, `sloc=11`, `mass=23.2163735324878` |
| `org.springframework.asm.ClassReader#readCode` (no varargs annotation; already measured pre-swap, `private` now wrapped in orchard's `modifier`/`visibility` node) | `cc=532`, `sloc=900`, `mass=15960.0` | `cc=532`, `sloc=900`, `mass=15960.0` (unchanged) |

Both `ClassUtils`/`ReflectionUtils` methods contain an `if`/`while`/`for`
(hence a nonzero post-swap `cc`) that was previously invisible to every
rule and score entirely — which is also where the `+2
JAVA-REDUNDANT-ELSE-AFTER-RETURN` findings and `+5` Java `flagged_lines`
come from: newly-measured callables across the other 53 files carrying
the same pattern, not a changed rule or a changed verdict on
already-measured code. `readCode` is the control for the other half of
that claim: it was already measured before the swap (it carries no
varargs-annotation damage), its `private` modifier is wrapped in orchard's
new `modifier`/`visibility` node exactly like every other Java
modifier keyword, and its `cc`/`sloc`/`mass` are bit-for-bit identical
before and after — no callable that was already measured before the swap
changed its `sloc`/`cc`/`mass` value; the entire delta is newly-measured
code that was previously invisible.

## Rows

Each row: date (UTC); script-appended rows add `(nsd@<full HEAD sha>)`,
suffixed `+dirty` when `git status --porcelain -- src Cargo.toml
Cargo.lock` is non-empty · machine · the two fixture repos and their
scanned shas · the scanner's own reported scanned-source-line count
(`overall` `verbosity.scanned_lines`) · wall-clock seconds
(`/usr/bin/time -l`'s `real`) · peak RSS in MB (`/usr/bin/time -l`'s
`maximum resident set size`, converted from bytes) · the scanner's own
`incomplete` flag · the scanner's own `skipped_files` count
(discovery-time exclusions plus non-`SyntaxError` parse failures;
`SyntaxError` files are salvaged and not listed; the seven
`2026-09-18`/`2026-09-23` rows predate this and also count the 58
`SyntaxError` files).

| date | machine | spring-framework | angular | scanned source lines | wall clock | peak RSS | incomplete | skipped files |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-09-18 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 17.54s | 1247.98 MB | true | 7720 |
| 2026-09-18 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 17.67s | 1248.16 MB | true | 7720 |
| 2026-09-23 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 4.28s | 1774.33 MB | true | 7720 |
| 2026-09-23 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 4.14s | 1775.42 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control, cold-build warm-up run, discarded from the ceiling comparison above; block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 6.90s | 1779.59 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control rerun 1, warm, block input operations: 0) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 6.77s | 1778.98 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control rerun 2, warm, block input operations: 0) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 5.68s | 1777.41 MB | true | 7720 |
| 2026-09-24 (M0c-10, `3dd9ae2` — control, `tree-sitter-java` 0.23.5, built in a separate scratch clone outside this tree so the swap's own Cargo.toml edit never touched this binary) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584349 | 4.52s | 1810.38 MB | true | 7662 |
| 2026-09-24 (M0c-10, this branch — `tree-sitter-java-orchard` 0.5.18) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.24s | 1822.98 MB | true | 7662 |
| 2026-09-28 (nsd@e21f629ccdd7f78d54bd9655147b4513ef6db4ed — contended: concurrent WS-5/6/7 cargo builds in this tree, discarded; block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 23.08s | 1732.75 MB | true | 7662 |
| 2026-09-28 (nsd@e21f629ccdd7f78d54bd9655147b4513ef6db4ed — contended: concurrent WS-5/6/7 cargo builds in this tree, discarded; block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 17.73s | 1500.23 MB | true | 7662 |
| 2026-09-28 (nsd@375b2c73e6eb34fb5f3792e5d40e1ed2dd9d9972 — contended: concurrent WS-5/6/7 cargo builds in this tree, discarded; block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 16.49s | 1596.62 MB | true | 7662 |
| 2026-09-28 (nsd@9d4e7a6429c635a4ffa128ff0d1d36f0ff11dd03 — block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.81s | 1836.67 MB | true | 7662 |
| 2026-09-28 (nsd@403aa7f74d545be0b567a212cfaebf8db8af2175 — block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.58s | 1840.81 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — contended: unrelated vitest workers running on the machine, discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 17.67s | 1837.86 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — contended: unrelated vitest workers running on the machine, discarded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 18.50s | 1837.72 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — discarded warm-up, contention just cleared) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.66s | 1840.75 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.33s | 1840.67 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-9 Part C reference, run 1/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.86s | 1838.64 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-9 Part C reference, run 2/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.48s | 1832.78 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-9 Part C reference, run 3/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 8.24s | 1841.38 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-9 Part C reference, run 4/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.59s | 1836.12 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-9 Part C reference, run 5/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.20s | 1838.20 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.38s | 1786.30 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — WS-9 C1 after, run 1/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.50s | 1787.19 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — WS-9 C1 after, run 2/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.93s | 1785.78 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — WS-9 C1 after, run 3/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.88s | 1785.08 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — WS-9 C1 after, run 4/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.31s | 1788.80 MB | true | 7662 |
| 2026-09-28 (nsd@8c020e0657f2907ba8f4db4c1ccd369d666da374 — WS-9 C1 after, run 5/5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.71s | 1787.23 MB | true | 7662 |
| 2026-09-28 (nsd@337d49bbfbf8aff8500dafdc17996b64b643c46f — WS-9 post-revert verification sanity run, not part of the Decision 11 before/after sets) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.58s | 1836.23 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.61s | 1836.75 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.84s | 1786.52 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A1) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.14s | 1840.88 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B1) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.78s | 1786.67 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.20s | 1838.78 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.99s | 1785.91 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.30s | 1843.20 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.87s | 1787.66 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.10s | 1838.31 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.72s | 1794.06 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A5, pair discarded: its paired B5 run below showed contention, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.17s | 1838.28 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B5, contended: OrbStack Helper CPU rose to ~34% during/after this run and cargo's own build check took 0.20s versus this session's usual 0.03-0.06s, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.71s | 1788.75 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.33s | 1838.11 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.74s | 1785.11 MB | true | 7662 |

## Part C (run 2026-09-25-1744-m1m2-followup)

WS-9 opens this section (task.md Part C: "Measure every change before and
after with `scripts/perf_scan.sh`, and record both in
`docs/measurements.md`. If a row's measured gain is below the noise
floor, revert it and record that; don't keep it on faith."). Decision 11
sets the accept rule every Part C row below follows: 5 warm
`perf_scan.sh` runs (a discarded warm-up after each fresh release build),
kept only when the before and after ranges do not overlap —
`max(after) < min(before)` on wall clock (or on peak RSS for a
memory-targeted row). Each stream measures its own parent commit; WS-9's
own before-set below is additionally frozen as the run's one fixed Part C
reference, so the cumulative trend across every later stream's own
before/after pair can be read against one fixed point instead of a
moving baseline.

**WS-9 (C1) reference — parent `079ca52`, before wiring one lowering into
all three readers.** The five `WS-9 Part C reference` rows in `## Rows`
above: 5.86s, 6.48s, 8.24s, 7.59s, 7.20s (min 5.86s, max 8.24s). Four
earlier runs on the same build and same commit — two "contended" (an
unrelated `vitest` workload sharing the machine) and two "discarded
warm-up" — are excluded from the reference per the same discipline; none
of the nine is cherry-picked, all nine are recorded in `## Rows` above
with the reason each was or was not kept.

**WS-9 (C1) after — `8c020e0`, one lowering per file wired into
`pipeline::run`, `metrics::run_with_ir`, `clones::run_with_ir` and
`rules::run_with_ir`.** The five `WS-9 C1 after` rows in `## Rows` above:
5.50s, 5.93s, 5.88s, 5.31s, 5.71s (min 5.31s, max 5.93s), after one
discarded warm-up on the same fresh release build. **Measured below the
noise floor, reverted 2026-09-28** (WS-9, revert commit follows this one):
Decision 11's accept rule is `max(after) < min(before)` on wall clock; here
`max(after) = 5.93s` is not less than `min(before) = 5.86s` -- the two
5-run ranges overlap by 0.07s, so this does not clear the noise floor by
the letter of the rule, even though the after-set's mean (5.67s) sits
visibly below the before-set's mean (7.07s) and four of its five runs sit
below all but the single fastest before-run. Per task.md Part C ("don't
keep it on faith") and this stream's own observable acceptance ("If the
measurement is below the noise floor, the Part C rule still applies: the
change, the seam and the test are reverted together"), the wiring
(`src/pipeline.rs`, `src/metrics/mod.rs`, `src/clones/mod.rs`,
`src/rules/mod.rs`), the `lower::lowering_count()` seam, and
`tests/ir_isolation.rs::test_pipeline_lowers_each_file_once` are reverted
together in the next commit rather than kept on this single overlapping
measurement. `docs/deferred-work.md`'s C1 row (WS-14's to edit, not this
stream's) stays open, annotated by WS-14 from this record.

**WS-9 (C1) round 2 — re-measured alternating, per the user's decision
(`questions.md`, `ws9-c1:` line) after round 1's before/after ranges
overlapped by only 0.07s on a noisy machine.** Round 1's reverted commit
(`337d49b`) was reverted forward (`24ecf8a`), restoring the wiring, the
`lowering_count()` seam and the mandatory test byte-identical to `8c020e0`
(`git diff 8c020e0 HEAD -- src tests` empty; `cargo test --locked --test
ir_isolation` green). Two detached worktrees were built in release mode:
`ws9-A` at `d61fefe` (src/tests byte-identical to parent `079ca52`,
without C1) and `ws9-B` at `24ecf8a` (with C1). Each worktree measures its
own binary and writes to its own copy of this file; both read the same
`$TMPDIR/nsd-perf-fixture`. After one discarded warm-up per side, 5 pairs
were run alternately (A1 B1 A2 B2 A3 B3 A4 B4 A5 B5), checking `ps -Ao
%cpu,comm -r` for contention before each run. The A5/B5 pair was visibly
contended: B5 read 4.71s against B's other four runs of 3.72-3.99s, its
own `cargo build` check took 0.20s against this session's usual
0.03-0.06s, and OrbStack Helper's CPU share climbed to ~34% immediately
after. Per this round's protocol the whole pair was re-run rather than
dropped; both the contended A5/B5 and the clean re-run are recorded above
under `## Rows`, labelled accordingly. The decision set is A1-A4 plus the
A5 re-run, and B1-B4 plus the B5 re-run: A = {4.14s, 4.20s, 4.30s, 4.10s,
4.33s} (min 4.10s, max 4.33s, mean 4.214s); B = {3.78s, 3.99s, 3.87s,
3.72s, 3.74s} (min 3.72s, max 3.99s, mean 3.82s). Decision 11's rule is
`max(after) < min(before)`: here `max(B) = 3.99s` is less than `min(A) =
4.10s`, a clean 0.11s gap with no overlap, unlike round 1's 0.07s
overlap -- so C1 clears the noise floor. Round 1's before-set was
measured on a noisier window (its own range spanned 5.86s-8.24s, 2.38s
wide, against this round's A-range of only 0.23s and B-range of only
0.27s), which is why round 1's sets are superseded by this round's
alternating measurement rather than trusted as-is; the frozen Part C
reference above is unchanged (Decision 11: it is not re-frozen). **Kept**:
the restore commit (`24ecf8a`) stands; no further revert follows.

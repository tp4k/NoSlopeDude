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
3 JS/TS parse failures contribute to it not at all, cleared or not.
(Greptile P2 on PR #1 put salvaged `SyntaxError` files back into
`skipped_files` as `salvaged` rows, so a row recorded after that change
counts them again.) The
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
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B5, contended: OrbStack Helper CPU climbed to ~34% immediately after this run and cargo's own build check took 0.20s versus this session's usual 0.03-0.06s, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.71s | 1788.75 MB | true | 7662 |
| 2026-09-28 (nsd@d61fefe58a0ede6166fe9a2febaaa35cf16d0877 — WS-9 r2 alternating, A5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.33s | 1838.11 MB | true | 7662 |
| 2026-09-28 (nsd@24ecf8acd6b95375cd713114a7d3e56def40b8a5 — WS-9 r2 alternating, B5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.74s | 1785.11 MB | true | 7662 |
| 2026-09-28 (nsd@997a9821cf2bb16e14c46a6cf7b1a0d5d7142d5a — WS-9 r2 post-restore verification sanity run in the primary worktree, not part of the Decision 11 alternating set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 8.12s | 1530.03 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.04s | 1782.94 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.48s | 1785.61 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A1) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.81s | 1788.78 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B1, contended: tsc at 130.8% CPU observed just before this run, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.88s | 1786.94 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A1 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.92s | 1778.88 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B1 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.10s | 1681.00 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.56s | 1783.50 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.59s | 1786.12 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.67s | 1784.12 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.58s | 1786.81 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.96s | 1532.45 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.79s | 1790.69 MB | true | 7662 |
| 2026-09-28 (nsd@eb26b40a4c36567444efb49356e311c637b0ab83 — WS-10 alternating, A5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.86s | 1779.22 MB | true | 7662 |
| 2026-09-28 (nsd@89f2a66075595414c942800676b1f2bbccc968cf — WS-10 alternating, B5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.40s | 1793.33 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) first measurement, superseded, A1; not retained) | not retained | not retained | not retained | not retained | 5.82s | not retained | not retained | not retained |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) first measurement, superseded, B1; not retained) | not retained | not retained | not retained | not retained | 5.42s | not retained | not retained | not retained |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) first measurement, superseded, A2; not retained) | not retained | not retained | not retained | not retained | 5.47s | not retained | not retained | not retained |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) first measurement, superseded, B2; not retained) | not retained | not retained | not retained | not retained | 5.70s | not retained | not retained | not retained |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) first measurement, superseded, A3; not retained) | not retained | not retained | not retained | not retained | 6.15s | not retained | not retained | not retained |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) first measurement, superseded, B3; not retained) | not retained | not retained | not retained | not retained | 5.37s | not retained | not retained | not retained |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) first measurement, superseded, A4; not retained) | not retained | not retained | not retained | not retained | 5.36s | not retained | not retained | not retained |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) first measurement, superseded, B4; not retained) | not retained | not retained | not retained | not retained | 5.59s | not retained | not retained | not retained |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) first measurement, superseded, A5; not retained) | not retained | not retained | not retained | not retained | 5.75s | not retained | not retained | not retained |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) first measurement, superseded, B5; not retained) | not retained | not retained | not retained | not retained | 5.45s | not retained | not retained | not retained |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.39s | 1781.22 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.72s | 1783.94 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A1) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.49s | 1785.31 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B1) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.49s | 1783.12 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.50s | 1783.66 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.34s | 1784.62 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.80s | 1789.59 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.46s | 1784.45 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.53s | 1783.19 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.65s | 1783.56 MB | true | 7662 |
| 2026-09-28 (nsd@49c464eeb1b9b2cfb3f470f1722a270f4cb63cf1 — WS-11 (C4) alternating, A5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.67s | 1783.08 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C4) alternating, B5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.42s | 1784.56 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, A-side discarded warm-up; A-side is `a9a363c`, C6's parent, unlike the C4 rows above where `a9a363c` was the "with" side; the sha cell reads the B-side commit, so which binary produced this warm-up is not established — it enters no decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.55s | 1713.56 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.44s | 1788.25 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A1; peak RSS not retained across this session's context compaction, wall clock is) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.38s | not retained | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B1; peak RSS not retained across this session's context compaction, wall clock is) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.24s | not retained | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A2; peak RSS not retained across this session's context compaction, wall clock is) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.40s | not retained | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B2; peak RSS not retained across this session's context compaction, wall clock is) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.33s | not retained | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.30s | 1784.95 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.25s | 1789.27 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A4, first attempt, contended: `ps` showed `node (vitest)` at 60.7% immediately before this run, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.94s | 1779.67 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B4, first attempt, contended: `ps` showed OrbStack Helper at 195.3% and `node (vitest)` at 89.1% immediately before this run, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.49s | 1787.22 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A4 re-run, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.59s | 1791.83 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B4 re-run, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.37s | 1789.70 MB | true | 7662 |
| 2026-09-28 (nsd@a9a363c17de776ac637c892e18a1ee17faf49691 — WS-11 (C6) alternating, A5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.44s | 1783.84 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C6) alternating, B5) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.17s | 1787.34 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.79s | 1787.67 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B-side discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.09s | 1793.42 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A1, first attempt) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.45s | 1786.70 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B1, first attempt, contended: result was a 7.51s outlier against this side's other four runs of 4.63-5.85s despite an unremarkable `ps` snapshot, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.51s | 1790.78 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A1 re-run, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.12s | 1793.70 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B1 re-run, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.19s | 1788.23 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.44s | 1783.62 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B2) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.66s | 1789.05 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.76s | 1787.66 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B3) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.70s | 1790.73 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A4, first attempt, contended: `ps` showed `node (vitest)` at 46.4% immediately before this run) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 13.24s | 1790.20 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A4 re-run, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.50s | 1786.75 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B4, used in the decision set — A4's contended first attempt was discarded before B4 was run, so only one B4 reading exists) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.63s | 1791.30 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A5, first attempt, contended: `ps` showed multiple `node (vitest N)` workers at 87-93% immediately before this run, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 12.43s | 1755.69 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A5, second attempt, still contended: `ps` showed multiple `node (vitest N)` workers at 58-75% immediately before this run, discarded, waited for contention to clear, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 13.61s | 1782.31 MB | true | 7662 |
| 2026-09-28 (nsd@4f3009d3a189451179bd0b4c648d3ed4dbe0f8fb — WS-11 (C7) alternating, A5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.93s | 1791.20 MB | true | 7662 |
| 2026-09-28 (nsd@eb8e4d71f0f4b3181d8e93816bd981a26747efa5 — WS-11 (C7) alternating, B5 re-run, contention cleared, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.85s | 1789.50 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X-side (with C7) discarded warm-up; ps top-2 before this run: WindowServer 24.1%, zen 21.5% (quiet)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.59s | 1786.55 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y-side (parent, without C7) discarded warm-up; ps top-2 before this run: OrbStack Helper 136.9%, zen 16.0% (no vitest/cargo/nsd)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.98s | 1781.83 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X1, used in the decision set; ps top-2 before this run: zen 29.3%, cmux 25.1%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.19s | 1787.05 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y1, used in the decision set; ps top-2 before this run: zen 18.6%, gpu-helper 16.0%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.58s | 1782.89 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X2, used in the decision set; ps top-2 before this run: zen 17.8%, WindowServer 15.5%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.37s | 1788.41 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y2, used in the decision set; ps top-2 before this run: Firefox plugin-container 33.9%, OrbStack Helper 23.4% (no vitest/cargo/nsd)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.58s | 1784.05 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X3, first attempt; ps top-2 before this run: zen 18.2%, gpu-helper 16.5% (unremarkable); the paired Y3 attempt read a 7.23s outlier, so this attempt is also discarded and the whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.98s | 1786.09 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y3, first attempt, contended: a 7.23s outlier against this side's other four decision-set runs (Y1/Y2/Y4/Y5: 5.55s-5.68s; the kept Y3 re-run, 6.15s, is also above that range, which works against C7) despite an unremarkable ps snapshot (OrbStack Helper 18.6%, WindowServer 17.8%) immediately before this run, discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 7.23s | 1782.39 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X3 re-run, used in the decision set; ps top-2 before this run: zen 15.2%, WindowServer 14.8%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.41s | 1592.97 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y3 re-run, used in the decision set; ps top-2 before this run: zen 17.3%, gpu-helper 14.9%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.15s | 1787.61 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X4, used in the decision set; ps top-2 before this run: WindowServer 24.9%, zen 18.9% — `node (vitest)` at 56.2% and `node (vitest 1)` at 49.3% had been observed just before and were allowed to clear first) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.43s | 1783.00 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y4, used in the decision set; ps top-2 before this run: OrbStack Helper 46.1%, zen 19.0% (no vitest/cargo/nsd)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.55s | 1786.88 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X5, first attempt; ps top-2 before this run: WindowServer 18.9%, zen 18.3% (clean; an earlier check had shown OrbStack Helper at 185.6% and this session waited 30s for it to clear before running); discarded because the paired Y5 attempt then read a contended ps — tsc at 229.3% CPU — before it could be run, so the whole pair was re-run per protocol before Y5's first attempt was ever measured) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.23s | 1788.44 MB | true | 7662 |
| 2026-09-28 (nsd@21a710f120df9192bcf49822a0b1d4da766f59a4 — WS-11 r2 (C7 re-measure) alternating, X5 re-run, used in the decision set; ps top-2 before this run: zen 17.0%, WindowServer 15.7%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.46s | 1778.44 MB | true | 7662 |
| 2026-09-28 (nsd@66787a07c0a30285cf20ef477e615a573ac0dba7 — WS-11 r2 (C7 re-measure) alternating, Y5 re-run, used in the decision set — X5's first attempt was discarded before Y5's first attempt was ever run, so only one Y5 reading exists; ps top-2 before this run: WindowServer 26.5%, zen 18.3%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.68s | 1782.66 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X-side (without C3) discarded warm-up; ps top-2 before this run: WindowServer 25.9%, zen 25.6%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.65s | 1785.20 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y-side (with C3) discarded warm-up; ps top-2 before this run: zen 17.7%, WindowServer 16.0%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.08s | 1787.97 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X1, used in the decision set; ps top-2 before this run: WindowServer 21.7%, zen 21.3%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.27s | 1788.78 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y1, used in the decision set; ps top-2 before this run: OrbStack Helper 46.5%, cmux 20.8% (no vitest/tsc/cargo/nsd)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.02s | 1788.95 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X2, first attempt, discarded because the paired Y2 attempt read contended; ps top-2 before this run: WindowServer 20.9%, zen 19.6%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.93s | 1788.45 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y2, first attempt, contended: this run's own `cargo build` check took 0.19s against this session's usual 0.06-0.16s, and a heavy unrelated `node` process (oxlint, an unrelated repo's linter) was observed at 132.9% CPU immediately afterward despite an unremarkable ps snapshot at the time (WindowServer 25.0%, plugin-container 17.1%), discarded, whole pair re-run per protocol) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.01s | 1786.36 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X2, second attempt (re-run), contended: ps showed an unrelated repo's `tsgolint` process at 454.5% CPU immediately before this run, discarded, whole pair re-run again before Y2 was ever measured a second time) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 6.49s | 1777.94 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X2, third attempt, used in the decision set; ps top-2 before this run: NotificationCenter 28.1%, zen 24.5%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.09s | 1785.05 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y2, second attempt, used in the decision set — X2's contended second attempt was discarded before Y2 was re-run, so only two Y2 readings exist; ps top-2 before this run: zen 15.4%, gpu-helper 13.7%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.38s | 1788.16 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X3, used in the decision set; ps top-2 before this run: OrbStack Helper 61.5%, cmux 27.6% (no vitest/tsc/cargo/nsd)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.49s | 1790.61 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y3, used in the decision set; ps top-2 before this run: git 50.1%, zen 15.7%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.02s | 1607.20 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X4, used in the decision set; ps top-2 before this run: Zen 15.1%, gpu-helper 13.0% (quiet, after this session waited roughly 4 minutes for a run of `node (vitest N)`/`tsc` workers on the shared machine to clear)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.29s | 1783.73 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y4, used in the decision set; ps top-2 before this run: OrbStack Helper 19.1%, zen 15.5%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.83s | 1788.55 MB | true | 7662 |
| 2026-09-28 (nsd@62de9bb3458cc3e1fd95c3d70122e7da6a326aca — WS-12 (C3) alternating, X5, used in the decision set; ps top-2 before this run: zen 15.6%, gpu-helper 12.4% (quiet, after a 15s wait for OrbStack Helper to drop from 113.6%)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.97s | 1788.98 MB | true | 7662 |
| 2026-09-28 (nsd@cce8f918aa1d97dc19f3050d8d6fd40a6e5915ac — WS-12 (C3) alternating, Y5, used in the decision set; ps top-2 before this run: zen 15.1%, OrbStack Helper 13.4%) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 5.65s | 1788.75 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A-side (frozen reference, before any Part C change) discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.51s | 1837.55 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B-side (final head, C1+C7 kept/C2+C3+C4+C6 reverted) discarded warm-up) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.49s | 1788.20 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A1, used in the decision set; ps top-2 before this run: OrbStack Helper 135.4%, WindowServer 42.2% (OrbStack Helper above one core; not re-run — A1 is A's max, so min(A) and the verdict are unaffected)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.25s | 1837.50 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B1, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.58s | 1784.89 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A2, used in the decision set; ps top-2 before this run: WindowServer 38.5%, gpu-helper 19.3% (quiet)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.10s | 1837.89 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B2, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.39s | 1787.16 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A3, used in the decision set; ps top-2 before this run: WindowServer 16.7%, gpu-helper 16.5% (quiet)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.00s | 1836.08 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B3, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.29s | 1785.23 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A4, used in the decision set; ps top-2 before this run: OrbStack Helper 24.0%, WindowServer 18.4% (quiet)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.09s | 1837.42 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B4, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.28s | 1785.73 MB | true | 7662 |
| 2026-09-28 (nsd@079ca5288bda6db24a26d0ecaca38e56c47639c8 — WS-14 (Part C cumulative) alternating, A5, used in the decision set; ps top-2 before this run: WindowServer 19.4%, StocksWidget 17.0% (quiet)) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 4.17s | 1791.02 MB | true | 7662 |
| 2026-09-28 (nsd@53eab9f54ed50b2db7965985a0cb314cfaa333e2 — WS-14 (Part C cumulative) alternating, B5, used in the decision set) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 584779 | 3.37s | 1785.23 MB | true | 7662 |

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
visibly below the before-set's mean (7.07s) and all five of its runs sit
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
overlap -- so C1 clears the noise floor. The verdict holds on A1-A4/B1-B4
alone, before either re-run is folded in: `max(B1-B4) = 3.99s` is still
less than `min(A1-A4) = 4.10s`. Including the contended pair as originally
read (B5 = 4.71s) would fail the rule instead -- `max(B with contended B5)
= 4.71s` is not less than `min(A) = 4.10s`. Round 1's before-set was
measured on a noisier window (its own range spanned 5.86s-8.24s, 2.38s
wide, against this round's A-range of only 0.23s and B-range of only
0.27s), which is why round 1's sets are superseded by this round's
alternating measurement rather than trusted as-is; the frozen Part C
reference above is unchanged (Decision 11: it is not re-frozen). **Kept**:
the restore commit (`24ecf8a`) stands; no further revert follows.

**WS-10 (C2) — alternating, parent `eb26b40` (worktree A, without C2)
versus `89f2a66` (worktree B, with C2: `KindIds` numeric kind/field id
tables replacing the string-based `node.kind()`/`child_by_field_name`
lookups in `src/lower/java.rs`/`src/lower/jsts.rs`/`src/exec_lines.rs`,
threaded through `classify`/`build_ir`/`lower_file`).** Two detached
worktrees were built in release mode under a scratch directory outside
this repo (`nsd-scratch/ws10-a` at `eb26b40`, `nsd-scratch/ws10-b` at
`89f2a66`), each measuring its own binary and writing to its own copy of
this file; both read the same `$TMPDIR/nsd-perf-fixture`. After one
discarded warm-up per side, 5 pairs were run alternately (A1 B1 A2 B2 A3
B3 A4 B4 A5 B5), checking `ps -Ao %cpu,comm -r` for contention before each
run. Only the B1 reading was retained; the other runs' `ps` readings were
not recorded. The A1/B1 pair was contended: a `tsc` process was observed at 130.8%
CPU immediately before B1 (it had appeared only after A1 itself had
already been measured clean). Per protocol the whole pair was re-run
rather than dropped; both the contended A1/B1 and the clean re-run are
recorded above under `## Rows`, labelled accordingly. The decision set is
the A1/B1 re-run plus A2-A5/B2-B5: A = {5.92s, 5.56s, 5.67s, 5.96s, 5.86s}
(min 5.56s, max 5.96s, mean 5.794s); B = {7.10s, 6.59s, 6.58s, 6.79s,
6.40s} (min 6.40s, max 7.10s, mean 6.692s). Decision 11's rule is
`max(after) < min(before)`: here `max(B) = 7.10s` is not less than
`min(A) = 5.56s` — the two ranges do not overlap at all, they are
disjoint in the wrong direction, with every single B run slower than
every single A run (B's own min, 6.40s, is still above A's own max,
5.96s). This is not a borderline overlap of the kind WS-9's round 1 saw;
it is a consistent, unambiguous regression of roughly 0.9s (~15.5%) per
scan, present in all five pairs including the initially-contended one.
Scanned-source-line count, `incomplete` and `skipped_files` match exactly
between A and B on every run (584779, true, 7662), confirming the
regression is a cost difference, not a correctness difference. Per
task.md Part C ("If a row's measured gain is below the noise floor,
revert it and record that; don't keep it on faith") and this stream's own
Decision 11 obligation, C2 fails the accept rule and is reverted forward
in the next commit rather than kept. Not measured as the cause:
`KindIds::build`'s per-file sweep. A release-mode serial `lower_file`
probe (best of 5, whole perf fixture, 8610 files, 8,034,787 nodes) put
parent `eb26b40` at 1.319s and `89f2a66` at 5.668s, a +4.35s CPU delta;
all 8610 `KindIds::build` calls together cost 0.087s (2% of that delta —
9.3µs per file for Java, 21.3µs for Tsx), so a per-`Language` cache would
recover at most that 2%. Measured as the cause: the cost is per lookup.
`KindIds::is` hashes its `&str` name into a `HashMap<&str, Vec<u16>>` on
every call (34.5–45.8ns, against 10.5–15.3ns for the
`node_kind_for_id(id) == Some(lit)` compare it replaced — 2.99–3.29× slower
per call), and each node tries several such calls before matching.
`89f2a66` was therefore not the best in-spec C2: a const-slot/id-indexed
variant that resolves each name to an integer slot once at build time,
leaving only an index or integer compare on the hot path, measured
1.210s/1.205s serial (about 8% faster than parent's 1.319s). Lowering is
about 4% of scan wall clock (about 0.25s of about 5.8s), so even that
best in-spec form saves only about 0.04s of wall — far below this
window's A-range spread (5.56s–5.96s). C2 is therefore recorded as not
clearing Decision 11 in any in-spec form, not only in the `89f2a66`
implementation measured above. Both scratch worktrees (`nsd-scratch/ws10-a`,
`nsd-scratch/ws10-b`) were removed via `git worktree remove --force`
after the last pair.

**WS-11 (C4) — alternating, parent `49c464e` (worktree A, without C4)
versus `a9a363c` (worktree B, with C4: an allocation-free fast path in
`ir_statement_tokens`'s anonymous-leaf branch in `src/clones/mod.rs`,
pushing `leaf_text` directly when the node is named or whitespace-free
instead of always going through
`split_whitespace().collect::<Vec<_>>().join(" ")`).** Two detached
worktrees were built in release mode under this session's scratch
directory, each measuring its own binary and writing to its own copy of
this file. After one discarded warm-up per side, 5 pairs were run
alternately (A1 B1 A2 B2 A3 B3 A4 B4 A5 B5), checking `ps -Ao %cpu,comm
-r` for contention before each run; none of the five pairs showed a
result inconsistent with its own side's other readings, so none was
re-run. No `ps` reading was recorded for any C4 run; the re-run decision
was made on the readings' consistency, not on `ps`. The decision set is A
= {5.49s, 5.50s, 5.80s, 5.53s, 5.67s} (min
5.49s, max 5.80s, mean 5.598s); B = {5.49s, 5.34s, 5.46s, 5.65s, 5.42s}
(min 5.34s, max 5.65s, mean 5.472s). Decision 11's rule is `max(after) <
min(before)`: here `max(B) = 5.65s` is not less than `min(A) = 5.49s` —
a 0.16s overlap. **Measured below the noise floor, reverted**: per
task.md Part C and this stream's own Decision 11 obligation, C4 does not
clear the rule and is reverted (`git revert --no-edit a9a363c`, commit
`077d9b1`) rather than kept on the strength of its lower mean. (This
supersedes an earlier same-day measurement of this pair whose per-run
peak-RSS figures were lost to this session's own context compaction
before they could be written here; that earlier measurement's wall-clock
decision sets — A = {5.82s, 5.47s, 6.15s, 5.36s, 5.75s}, B = {5.42s,
5.70s, 5.37s, 5.59s, 5.45s} — reached the same verdict, `max(B) = 5.70s`
not less than `min(A) = 5.36s`, and are not the rows recorded in `##
Rows` above; their warm-up readings were not retained; the rows above are
this fresh, fully-verified re-measurement
against the same two commits.) Both scratch worktrees were removed via
`git worktree remove --force` after the last pair.

**WS-11 (C6) — alternating, parent `a9a363c` (worktree A, without C6, the
same commit that is C4's own "with" side above) versus `4f3009d`
(worktree B, with C6: an `if damage.is_empty()` fast path in
`cascade_exclusions`, plus the same short-circuit around the
`redact_targets` scans and `kept_callables`/`kept_blocks` rebuild in
`lower_file`, `src/lower/mod.rs`).** `4f3009d`'s fast path did not skip
WS-4's owner compaction and remap (`src/lower/mod.rs:224-242` at
`4f3009d`), which the plan's reading includes; all of serial `lower_file`
is 1.32 s of CPU (review-ws11-r1-perf.md) and at most 0.09 s of wall at 15
threads, so including it cannot change the verdict. Two detached
worktrees were built in release mode under this session's scratch
directory. After one discarded warm-up per side (5.55s/1713.56 MB; the
sha cell reads the B-side commit, so which binary produced this warm-up
is not established — it enters no decision set, 5.44s/1788.25 MB), 5
pairs were run alternately, checking `ps -Ao %cpu,comm -r` for contention
before each run. Only the `ps` readings quoted below were retained; the
other runs' readings were not recorded. The A4/B4 pair was contended on
its first attempt — `ps` showed
`node (vitest)` at 60.7% before A4 and OrbStack Helper at 195.3% plus
`node (vitest)` at 89.1% before B4 — and was re-run per protocol; both
attempts are recorded in `## Rows` above. Peak RSS for A1/B1/A2/B2 was
lost to this session's own context compaction and is recorded above as
"not retained" rather than invented; their wall-clock readings survived
in this session's own running record and are used below. The decision
set is A = {5.38s, 5.40s, 5.30s, 5.59s (A4 re-run), 5.44s} (min 5.30s,
max 5.59s, mean 5.422s); B = {5.24s, 5.33s, 5.25s, 5.37s (B4 re-run),
5.17s} (min 5.17s, max 5.37s, mean 5.272s). Decision 11's rule is
`max(after) < min(before)`: here `max(B) = 5.37s` is not less than
`min(A) = 5.30s` — a 0.07s overlap, the same size as WS-9's round-1
overlap. **Measured below the noise floor, reverted**: C6 does not clear
Decision 11 and is reverted (`git revert --no-edit 4f3009d`, commit
`77ec2cd`) rather than kept on the strength of its lower mean. Both
scratch worktrees were removed via `git worktree remove --force` after
the last pair.

**WS-11 (C7) — alternating, parent `4f3009d` (worktree A, without C7, the
same commit that is C6's own "with" side above) versus `eb8e4d7`
(worktree B, with C7: a per-`aggregate()`-call `HashMap<PathBuf,
Vec<String>>` cache threaded through `read_excerpt`/`build_finding`/
`build_duplicate_group`/`build_callable`/`build_location`,
`src/report/mod.rs`, so a file's lines are read and split once per
`report::run` call instead of once per location).** Two detached
worktrees were built in release mode under this session's scratch
directory. After one discarded warm-up per side, 5 pairs were run
alternately, checking `ps -Ao %cpu,comm -r` for contention before each
run. Only the `ps` readings quoted below were retained; the other runs'
readings were not recorded. Three of the five pairs needed a re-run: B1's
first attempt (7.51s)
was a clear outlier against B's other four runs (4.63s-5.85s) despite an
unremarkable `ps` snapshot at the time, so the whole A1/B1 pair was
re-run; A4's first attempt (13.24s) coincided with `node (vitest)` at
46.4% CPU, so the whole A4/B4 pair was re-run (B4 itself only has one
reading, since A4's contended attempt was discarded before B4 was
measured); A5's first two attempts (12.43s, 13.61s) coincided with
multiple `node (vitest N)` workers at 58-93% CPU, so this session waited
roughly 50s for the load to visibly clear (confirmed by a fresh `ps`
snapshot showing only ordinary desktop processes under 35% CPU) before
re-running the whole A5/B5 pair a third time. All contended and clean
attempts are recorded in `## Rows` above. The decision set is A = {6.12s
(A1 re-run), 5.44s, 5.76s, 5.50s (A4 re-run), 5.93s (A5 re-run)} (min
5.44s, max 6.12s, mean 5.75s); B = {5.19s (B1 re-run), 5.66s, 4.70s,
4.63s (B4), 5.85s (B5 re-run)} (min 4.63s, max 5.85s, mean 5.206s).
Decision 11's rule is `max(after) < min(before)`: here `max(B) = 5.85s`
is not less than `min(A) = 5.44s` — a 0.41s overlap; the conclusion is
unaffected by A1's own contested value, since even discarding A1 entirely
the remaining A-set's min (5.44s, from A2) is still below B's max. Despite
B's noticeably lower mean (5.206s versus A's 5.75s), Decision 11's strict
non-overlap rule is not met on this noisy window. **Measured below the
noise floor, reverted**: C7 does not clear Decision 11 and is reverted
(`git revert --no-edit eb8e4d7`, commit `c2da01e`) rather than kept on
the strength of its lower mean. Both scratch worktrees were removed via
`git worktree remove --force` after the last pair.

**WS-11 round 2 (C7 re-measure) — the round-1 revert above was wrong.**
review-ws11-r1-perf.md's stage probe found `report::aggregate` taking
0.621-0.657s at the parent and 0.069-0.072s with C7 (a 0.56s saving on
the serial path, matching the wall-mean delta of -0.54s above), which is
larger than any of this session's quiet-window range widths (0.29-0.40s).
Per the triage's fix, C7 was restored with `git revert --no-edit
c2da01e` (commit `21a710f`, reverting the revert; `git diff --stat 49c464e
HEAD -- src tests` reads the same 124 insertions/15 deletions as the
original C7 commit) and re-measured with the WS-9 r2 alternating protocol
in a quiet window. Two detached worktrees were built in release mode:
`ws11c7-x` at `21a710f` (with C7) and `ws11c7-y` at `66787a0` (the
revert-commit's parent, without C7; src/tests byte-identical to
`49c464e`) — the parent placed in `ws11c7-y`, the path playing the role
`ws11-c7-B` held last round (C7's own "with" side then), so any
worktree-order bias works against C7 this time (perf note 3). Both were
built with `cargo build --release --locked` and each measures its own
binary, writing to its own copy of this file; both read the same
`$TMPDIR/nsd-perf-fixture`. Before starting, `ps -Ao %cpu,comm -r` showed
OrbStack Helper at 158.5%; after a 20s wait it read WindowServer 24.1%,
zen 21.5%, with no `node (vitest…)`, no `cargo`/`target/*/deps/*` and no
other `nsd` process, so the window counted as quiet. After one discarded
warm-up per side, 5 pairs were run alternately (X1 Y1 X2 Y2 X3 Y3 X4 Y4
X5 Y5), checking `ps -Ao %cpu,comm -r` for contention before each run; its
top-2 line is recorded with every row above. Three pairs needed a re-run:
Y3's first attempt (7.23s) was a clear outlier against Y's other four
decision-set runs (Y1/Y2/Y4/Y5: 5.55s-5.68s; the kept Y3 re-run, 6.15s, is
also above that range, which works against C7) despite an unremarkable `ps` snapshot at
the time (matching round 1's own B1 precedent), so the whole X3/Y3 pair
was re-run; before X4 ran, `ps` showed `node (vitest)` at 56.2% and `node
(vitest 1)` at 49.3%, so this session waited about 50s for the load to
clear (confirmed by a fresh, quiet `ps` snapshot) before running that
pair; before Y5 could run, `ps` showed `tsc` at 229.3% CPU (X5's own
first attempt, at 5.23s, had itself run against a clean `ps` snapshot),
so the whole X5/Y5 pair was re-run before Y5's first attempt was ever
measured, leaving X5 with two readings and Y5 with one, the same
asymmetry round 1's own A4/B4 pair showed. All contended and clean
attempts are recorded in `## Rows` above. The decision set is X (with C7)
= {5.19s, 5.37s, 5.41s (X3 re-run), 5.43s, 5.46s (X5 re-run)} (min 5.19s,
max 5.46s, mean 5.372s); Y (parent, without C7) = {5.58s, 5.58s, 6.15s
(Y3 re-run), 5.55s, 5.68s (Y5 re-run)} (min 5.55s, max 6.15s, mean
5.708s). Decision 11's rule is `max(after) < min(before)`: here `max(X) =
5.46s` **is** less than `min(Y) = 5.55s` — a clean 0.09s gap, no overlap,
unlike round 1's 0.41s overlap on a noisier window. **Kept**: C7 clears
Decision 11 on this quiet-window re-measurement; the restore commit
(`21a710f`) stands and no further revert follows. Both scratch worktrees
(`ws11c7-x`, `ws11c7-y`) were removed via `git worktree remove --force`
after the last pair.

C4 and C6 end up reverted: neither clears Decision 11's strict
`max(after) < min(before)` rule on this session's noise floor, even
though both showed a lower mean on the "after" side (C4's after-mean was
5.472s against a before-mean of 5.598s; C6's was 5.272s against 5.422s).
`49c464e`'s release binary is code-identical to `24ecf8a`'s, which WS-9
r2 measured at 3.72-3.99s; C4's A side read 5.49-5.80s here, so all three
WS-11 round-1 windows ran about 45% slower than WS-9 r2's. The round-2 C7
window was no faster: its Y side (`66787a0`, code-identical to `24ecf8a`)
read 5.55-6.15s, about 50% slower than WS-9 r2's; "quiet" in the round-2
paragraph means only the triage's `ps` criteria (no `node (vitest…)`,
`cargo`/`target/*/deps/*` or other `nsd` process). C7, unlike C4
and C6, clears Decision 11 in the quiet-window round-2 re-measurement
above and is kept. Of the Part C rows measured so far, only C1 (WS-9
round 2) and C7 (WS-11 round 2) have cleared Decision 11.

A stage-level probe (review-ws11-r1-perf.md; release build, best of 5,
whole perf fixture) explains why neither C4's nor C6's lower wall-clock
mean is a real gain. Serial `lower_file` over all 8610 files took
1.336/1.317/1.337s at the parent and 1.319/1.320/1.332s with C6; at
`RAYON_NUM_THREADS=1`, `lower_all` took 1.325/1.313s against 1.300/1.309s.
C6 therefore saves at most 0.02s of CPU (only 3 of the 8610 files have
non-empty `damage`, so the fast path is taken almost everywhere and still
saves almost nothing), spread over 15 rayon threads — about 0.02s / 15 ≈
1ms of wall. That is roughly 10x smaller than C6's own -0.15s mean wall
delta (5.422s to 5.272s), so that wall-clock difference is a
worktree/order artifact, not a gain. `clones::run_with_ir` at one thread
took 3.861/3.895s at the parent against 3.682/3.705s with C4, a saving of
about 0.19s of CPU inside `enumerate_candidates`'s `par_iter`
(`src/clones/mod.rs:155-165`, about 0.19s / 15 ≈ 0.013s of wall) — but at
full threads the stage read 2.920/3.002/2.862s at the parent against
2.977/3.002/2.879s with C4, so no change is visible there either. Neither
C4 nor C6 can clear Decision 11 in any window; their reverts stand.

**WS-12 (C3) — alternating, parent `62de9bb` (worktree X, without C3)
versus `cce8f91` (worktree Y, with C3: a repo-wide `[profile.release]`
table setting `lto = "thin"`, to restore cross-crate inlining for
`blake3::Hasher::update`/`finalize` in the hot push loop, task.md Part
C via triage-ws3-r1).** `Cargo.lock` is unaffected by this change (`git
diff --quiet HEAD~1 -- Cargo.lock` on the C3 commit exits 0). Two
detached worktrees were built in release mode under this session's
scratch directory (`ws12-x` at `62de9bb`, `ws12-y` at `cce8f91`), each
measuring its own binary and writing to its own copy of this file; both
read the same `$TMPDIR/nsd-perf-fixture`. Release build time and binary
size (context, not part of the verdict): X's `cargo build --release
--locked` took 18.77s real and produced a 7,812,816-byte binary; Y's
took 18.14s real and produced a 7,847,136-byte binary (34,320 bytes
larger; measured, not inferred: `__text` grew from 2,024,148 to
2,090,720 bytes per `size -m`, `<blake3::Hasher>::finalize` is a
standalone symbol at `62de9bb` but not at `cce8f91`, and
`<blake3::Hasher>::update` is still standalone in both, per `nm -C` —
review-ws12-r1-code.md) — the two builds were run back-to-back and are
not a controlled comparison of link time itself. Before starting, `ps
-Ao %cpu,comm -r` was checked and found quiet (no `node (vitest…)`,
`tsc`, `cargo`/`target/*/deps/*` or other `nsd` process). After one
discarded warm-up per side, 5 pairs were run alternately (X1 Y1 X2 Y2
X3 Y3 X4 Y4 X5 Y5), checking `ps -Ao %cpu,comm -r` for contention before
each run; its top-2 line is recorded with every row above. One pair
needed two re-runs: Y2's first attempt (6.01s) coincided with its own
`cargo build` check taking 0.19s (against this session's usual
0.06-0.16s) and a heavy unrelated `node` process (another repo's
`oxlint` linter) observed at 132.9% CPU immediately afterward, despite
an unremarkable `ps` snapshot at the time — the same "slow build check,
quiet-looking ps" signature WS-9 r2's own B5 used to flag contention —
so the whole X2/Y2 pair was re-run; that re-run's X2 attempt (6.49s)
itself coincided with an unrelated repo's `tsgolint` process at 454.5%
CPU immediately before it ran, so the pair was re-run a second time
before Y2 was measured again, leaving X2 with three readings and Y2
with two, the same asymmetry WS-11's C7 round-1 A4/B4 and round-2
X5/Y5 pairs showed.
Before X4, this session waited roughly 4 minutes for a prolonged run of
`node (vitest N)` and `tsc` workers on the shared machine to clear,
confirmed by a fresh quiet `ps` snapshot, before running that pair (no
mid-run contention was observed once it started, so neither X4 nor Y4
was itself re-run). All contended and clean attempts are recorded in
`## Rows` above. The decision set is X (without C3) = {5.27s, 5.09s
(third attempt), 5.49s, 5.29s, 4.97s} (min 4.97s, max 5.49s, mean
5.222s); Y (with C3) = {5.02s, 5.38s (second attempt), 5.02s, 4.83s,
5.65s} (min 4.83s, max 5.65s, mean 5.18s). Decision 11's rule is
`max(after) < min(before)`: here `max(Y) = 5.65s` is not less than
`min(X) = 4.97s` — a 0.68s overlap, far wider than any other Part C
row's overlap in this run (the next-widest, WS-11's round-1 C7,
was 0.41s), and the two means (5.18s vs 5.222s, ~0.8% apart) sit well
inside that overlap rather than on either side of it. **Measured below
the noise floor, reverted**: per task.md Part C ("If a row's measured
gain is below the noise floor, revert it and record that; don't keep it
on faith") and this stream's own Decision 11 obligation, C3 does not
clear the accept rule and is reverted by `4aceb37` (a revert of the
reapply `ccb948e` — the first revert and its own reapply carried no
`Co-Authored-By` trailer because `git revert --no-edit`'s default
message was used, and `git commit --amend` is forbidden, so the fix
was a further forward revert/reapply/revert cycle rather than an edit
to the two earlier commits; `4aceb37` is the one that stands, with the
trailer, and its tree is byte-identical to `62de9bb`'s `Cargo.toml`)
rather than kept on the strength of a mean difference this small. A
stage-level probe (review-ws12-r1-perf.md) does explain the overlap:
the `blake3` inlining mechanism named in task.md's C3 line is refuted
for `update` and holds only for `finalize`. `<blake3::Hasher>::update`
stays an out-of-line symbol under thin LTO, with the same 5 `bl` call
sites in `clones::run_with_ir::{closure#0}` on both sides; `finalize`
is gone because it was inlined, but it was only a wrapper, so the
closure now calls `final_output` and `compress_in_place` directly.
At 1 thread, the stage probe measured `lower_all` at 1.37s → 1.25s
(a 0.11s saving) and `clones::run_with_ir` at ≤ 0.03s saved. At full
threads, whole-scan `user` CPU in the upper group read 9.34-9.45s at
the parent and 9.12-9.23s with C3, about 0.2s of CPU (about 2%),
nearly all inside `par_iter` stages; spread across 15 cores that is
about 0.02-0.05s of wall time — an order of magnitude below this
run's 0.29-0.40s quiet-window noise band, so no measurement window
can clear it. `lto = "thin"` plus `codegen-units = 1` was also
probed, within the same `[profile.release]` table: `update` still has
34 `bl` sites out-of-line, the saving is at most about 0.1s of
parallel CPU, and the binary is 7.04 MB.
Both scratch worktrees (`ws12-x`, `ws12-y`) were removed via `git
worktree remove --force` after the last pair.

WS-14 closes this section with the mandatory final-head-vs-frozen-reference
cumulative measurement (task.md A3), using the same alternating protocol as
WS-9 r2/WS-10/WS-11 r2/WS-12 above: two scratch worktrees under this
session's scratchpad, `ws14-A` at `079ca52` (the frozen Part C reference,
WS-8's last commit, before any Part C change) and `ws14-B` at `53eab9f`
(this run's final head, with C1 and C7 kept and C2, C3, C4 and C6
reverted), each built with `cargo build --release --locked` and reading
the same `$TMPDIR/nsd-perf-fixture`. One discarded warm-up per side, then
5 alternating pairs (A1 B1 … A5 B5), with a `ps -Ao %cpu,comm -r` top-2
check before each pair; A1's check showed OrbStack Helper above one core
(135.4%) — not re-run, because A1 is A's max, so min(A) and the verdict
are unaffected — and the other four checks were quiet, with no
`vitest`/`tsc`/`cargo`/`nsd` contender, so no pair was re-run. Decision
set: A (frozen reference) = {4.25s, 4.10s, 4.00s, 4.09s, 4.17s} (min
4.00s, max 4.25s, mean 4.122s); B (final head) = {3.58s, 3.39s, 3.29s,
3.28s, 3.37s} (min 3.28s, max 3.58s, mean 3.382s). Decision 11's rule is
`max(after) < min(before)`: here `max(B) = 3.58s` is less than
`min(A) = 4.00s`, a clean 0.42s gap with no overlap, so the cumulative
Part C change clears the accept rule (mean speedup 4.122s / 3.382s ≈
1.22×). The A side here is a same-window re-measure of `079ca52`, not
WS-9 r2's frozen before-set ({5.86s, 6.48s, 8.24s, 7.59s, 7.20s}, mean
7.07s), which came from a slower window; `max(B) = 3.58s` also clears
that frozen set's `min = 5.86s`. Identical release code (`62de9bb`,
WS-12's X set) measured 4.97-5.49s (mean 5.22s) in WS-12's own window, so
between-window swings here reach about 1.8s and only the within-window
ratio (≈1.22×) is meaningful. This window agrees with WS-9 r2's: its
`d61fefe` A set, code-identical to `079ca52`, read mean 4.21s against
this window's 4.12s. This is a wall-clock outcome only, not a per-change attribution:
C1 (kept, `24ecf8a`) and C7 (kept, `21a710f`) each individually cleared
Decision 11 in their own rounds (WS-9 r2, WS-11 r2), while C2, C3, C4 and
C6 were each individually reverted for failing it, so this cumulative
number is consistent with — but does not re-derive — those per-item
verdicts. Both scratch worktrees (`ws14-A`, `ws14-B`) were removed via
`git worktree remove --force` after the last pair.

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

The rows below come from an **incomplete** scan: at these pins, the
scanner's own `report.json` marks `incomplete: true` (the row's own
`incomplete` cell) because 58 files fail to parse (55 Java, 3 JS/TS) and
are dropped whole-file from every score. The row's `skipped files` cell is
`report.json`'s full `skipped_files` length, which also folds in this
run's discovery-time exclusions (test directories, generated code,
dependency/build output under D16's default rules) alongside those 58
parse failures — it is not itself the parse-failure count, only an upper
bound on it. The parse failures' cause is a `tree-sitter-java` 0.23.5
grammar limitation on a type-use annotation on a varargs parameter (e.g.
`Class<?> @Nullable ... cs`) plus an unrelated `tree-sitter-typescript`
0.23.2 gap on a parameter literally named `using` — filed as a refactor
request against WS-2, not fixed in this stream (see this round's
implementer report). All rows were taken with a **warm page cache**
(`/usr/bin/time -l`'s own `block input operations: 0`), so wall-clock
seconds do not include first-touch disk I/O. Across these two fixtures,
peak RSS scales roughly linearly with total source bytes at **≈27×** (the
pipeline retains every parsed tree and source file simultaneously rather
than streaming file-by-file), so extrapolating to a much larger corpus
should scale the RSS estimate by source-byte count, not by scanned-line
count alone.

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
`2026-09-23` rows above measure `e83228b` (the fusion commit) via two
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
sessions ~20 minutes to hours apart, never back-to-back) — this file
records what was measured, not a settled attribution of the fusion's own
effect.

## Rows

Each row: date (UTC) · machine · the two fixture repos and their scanned
shas · the scanner's own reported scanned-source-line count (`overall`
`verbosity.scanned_lines`) · wall-clock seconds (`/usr/bin/time -l`'s
`real`) · peak RSS in MB (`/usr/bin/time -l`'s `maximum resident set
size`, converted from bytes) · the scanner's own `incomplete` flag ·
the scanner's own `skipped_files` count (parse failures and discovery-time
exclusions combined).

| date | machine | spring-framework | angular | scanned source lines | wall clock | peak RSS | incomplete | skipped files |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-09-18 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 17.54s | 1247.98 MB | true | 7720 |
| 2026-09-18 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 17.67s | 1248.16 MB | true | 7720 |
| 2026-09-23 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 4.28s | 1774.33 MB | true | 7720 |
| 2026-09-23 | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 4.14s | 1775.42 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control, cold-build warm-up run, discarded from the ceiling comparison above; block input operations not recorded) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 6.90s | 1779.59 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control rerun 1, warm, block input operations: 0) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 6.77s | 1778.98 MB | true | 7720 |
| 2026-09-23 (a370e3d, fusion's parent commit — control rerun 2, warm, block input operations: 0) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 5.68s | 1777.41 MB | true | 7720 |

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
a control until the row appended below labeled `a370e3d`: the two
existing unlabeled `2026-09-23` rows above it measure `e83228b` (the
fusion commit) itself, and the `a370e3d` row measures the fusion's
immediate parent commit, built in a detached worktree and run back to
back on the same machine against the same already-fetched fixture root
(the same pattern `scripts/neutrality_gate.sh` uses for its own pre-IR
reference build) — this pair is the only one in this file that isolates
the fusion's own effect from the rest of the M0b branch.

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
| 2026-09-23 (a370e3d, fusion's parent commit — control) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 570647 | 6.90s | 1779.59 MB | true | 7720 |

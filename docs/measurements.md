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

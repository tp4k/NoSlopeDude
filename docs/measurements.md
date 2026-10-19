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
scan of that root under `/usr/bin/time -l` and inserts a row at the end of the `## Rows` table below.
`scripts/perf_check.sh` does the same for `nsd check` (cold and warm time,
cache size) into the `## Check rows` table at the end of this file.

The fixture root lives **outside this repo's tree** (not a gitignored
in-repo directory): the scanner's own walk honours a parent `.gitignore`
by default (`require_git(false)`, `WalkBuilder::parents`), so an ignored
in-repo path would be invisible to the scan itself and the measurement
would silently run against an empty tree. Both scripts write this file,
`docs/measurements.md`, and `insert_table_row` stages the new copy as
`docs/measurements.md.tmp.$$` before renaming it over the old one.
`perf_scan.sh` always runs `cargo build --release`, and `perf_check.sh` runs it
unless given a binary argument, so both may write `target/`; nothing else under
this repository is written.

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

## Known caveats of the recorded rows

- The two `2026-09-18` rows are `agent_slope`'s own pre-import numbers
  (imported with `37829cb` at upstream `912ec7a`); their delta to the
  `2026-09-23` rows spans the whole M0b IR branch and is attributable to no
  single commit.
- Rows through `2026-09-23` drop 58 whole files (55 Java, 3 TS) on
  `SyntaxError`, so their `skipped files` cell reads 7720 (`7662 + 58`). From
  the `M0c-10` orchard row onward the 55 Java failures are gone (*M0c-10*
  below) and salvage keeps the 3 TS files, excluding only their damaged
  callables: `incomplete: true` remains but no longer means a whole-file drop,
  and `skipped files` counts discovery-time exclusions only (plus `salvaged`
  rows once Greptile P2 on PR #1 landed).
- `verbosity.scanned_lines` counts analyzed executable lines. The rows predate
  M1-8's `unanalyzed_lines` and `complete` and were not re-measured.
- Every row not marked `block input operations not recorded` was taken with a
  warm page cache. Peak RSS scales roughly linearly with total source bytes at
  about 27x, so extrapolate RSS by source bytes, not by scanned lines.
- The `2026-09-28` WS-9 to WS-14 rows are alternating A/B runs from the Part C
  per-change performance experiments of run 2026-09-25-1744-m1m2-followup;
  their per-change verdicts are in this file's history (at `cb33206`).

## M0c-10: the Java grammar swap's own measured effect

The two `2026-09-24` (`M0c-10`) rows in `## Rows` scan the same fixture at the
same pins: a control built at `3dd9ae2` (`tree-sitter-java` 0.23.5, outside
this tree) and the orchard 0.5.18 build. `tests/grammar_gate.rs` reads
`PipelineOutput::parse_failures` (not `skipped_files`, which filters
`SyntaxError` out): 55 Java `SyntaxError` failures under 0.23.5, all the
`Class<?> @Nullable ... cs` varargs-annotation misparse, become **0** under
orchard; the 3 TS failures on a parameter named `using` remain.

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

The `js_ts` rows are bit-for-bit identical, as required. The whole Java delta
is fail-closed damage exclusion lifting: a callable whose parameters held the
varargs `ERROR` node was excluded entirely before the swap and is fully
measured after it; orchard's `modifier`/`visibility` wrapper moves nothing,
since `is_executable_leaf` never accepts a node with children. Hand-checked
callables (pinned public fixture):

| callable | before (0.23.5) | after (orchard) |
| --- | --- | --- |
| `org.springframework.util.ClassUtils#getMethodIfAvailable` (`Class<?> clazz, String methodName, @Nullable Class<?> @Nullable ... paramTypes`) | excluded: file `incomplete=true`, callable absent from every measurement (`sloc`/`cc`/`mass` = 0/unmeasured) | `cc=3`, `sloc=8`, `mass=8.485281374238571` |
| `org.springframework.util.ReflectionUtils#findMethod` (`Class<?> clazz, String name, Class<?> @Nullable ... paramTypes`) | excluded: file `incomplete=true`, callable absent from every measurement | `cc=7`, `sloc=11`, `mass=23.2163735324878` |
| `org.springframework.asm.ClassReader#readCode` (no varargs annotation; already measured pre-swap, `private` now wrapped in orchard's `modifier`/`visibility` node) | `cc=532`, `sloc=900`, `mass=15960.0` | `cc=532`, `sloc=900`, `mass=15960.0` (unchanged) |

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
| 2026-10-13 (nsd@bce31e41f88aa9b6021ad74e80a515472d95ebb4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 583701 | 3.37s | 1821.20 MB | true | 7715 |
| 2026-10-13 (nsd@3b2c40e29155521fa422f3cb46b940e46dcc9b02 — second independent run, same plain-tree fixture) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 583701 | 3.32s | 1821.91 MB | true | 7715 |

The two 2026-10-13 rows scan a plain copy of both clones without their `.git`
entries (`scripts/perf_scan.sh` `build_plain_tree`), because the nested-checkout
rule would otherwise exclude both clones. Against the 2026-09-28 rows,
`scanned_lines` is -1078 and `skipped files` is +53. Measured against the
pre-WS-3 binary (`03f2192`) on the same fixture (584779 lines, 7665 skipped),
WS-3 adds exactly 50 `test` rows and removes none: 43 under `**/e2e/**` (9 of
them also match `*.e2e-spec.*`) and 7 matching only `*.e2e-spec.*`; none comes
from the `**/fixtures/**`, `**/fixture/**`, `**/__fixtures__/**`, `**/qa/**` or
WebJar globs. All -1078 lines are js_ts (294902 -> 293824; java unchanged at
289877). The remaining +3 skipped against 2026-09-28 predate WS-3 (`03f2192`
already reports 7665). These attribution numbers were measured by the WS-3
round-2 reviewer and not re-derived in round 3.

## Check rows

`scripts/perf_check.sh` (WS-3, `nsd-plan-final.md` *Fixtures and end-to-end*:
"record cold/warm time, peak RSS, cache size") inserts one row at the end of
the table below. It builds a plain copy of both fixture clones (the same
`build_plain_tree` as `perf_scan.sh`) in a scratch directory, `git init`s it,
commits it as the base, commits one small new Java file as the candidate, and
runs `nsd check --base HEAD~1 --format json` from that repo root twice under
`/usr/bin/time -l`: cold (no `<git dir>/nsd` cache), then warm. The check never
runs inside the fixture clones. The script refuses to write a row when the
fixture is missing, when either run is not a completed check (process status
outside `0`-`3`, or stdout not one check JSON document whose `exit_status`
equals that status), or when the cold and warm JSON outputs differ.

Each row: date (UTC) with `(nsd@<full HEAD sha>)`, suffixed `+dirty` when
`git status --porcelain -- src Cargo.toml Cargo.lock` is non-empty · machine ·
the two fixture repos and their scanned shas · the candidate (one new `.java`
file, so the check analyses one added file against the whole base) · cold wall
seconds and cold peak RSS in MB · warm wall seconds and warm peak RSS in MB
(`/usr/bin/time -l`'s `real` and `maximum resident set size`) · cache size in
MB after the warm run (`du -sk` of `<git dir>/nsd/cache`, so kilobyte blocks
divided by 1024, not apparent bytes) · the cold run's check exit status (the
warm run's `exit_status` is part of the compared JSON). Exit status 2 on the 2026-10-15 row is the check's own result,
not a script failure: its one diagnostic is `NSD-A102` (`too_large`) for
`angular/.github/actions/deploy-docs-site/main.js`, a file the base already
carries; the candidate file raises nothing.

| date | machine | spring-framework | angular | candidate | cold wall clock | cold peak RSS | warm wall clock | warm peak RSS | cache size | exit status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-15 (nsd@4a0cd37d69595466692aee1354bcdf77f8992dc2) | Darwin 27.0.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 1 new .java file | 3.37s | 930.08 MB | 1.68s | 867.88 MB | 187.99 MB | 2 |

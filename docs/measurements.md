# Scan and check measurements

These measurements are informational, not release gates. The fixture is a
plain copy of two public repositories at fixed revisions:

- Spring Framework: `e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9`
- Angular: `a783c4e7b753929ababa610e305112b82aaa0eb0`

Fetch the fixture with `scripts/fetch_perf_fixture.sh`. On macOS,
`scripts/perf_scan.sh` runs a release scan and appends a row to **Rows**;
`scripts/perf_check.sh` checks a one-file candidate twice and appends a row
to **Check rows**. Both scripts require the fixture and refuse to record an
incomplete command run. They write this file, so inspect the new row before
committing it. Earlier measurements and their attribution notes remain in Git
history.

## Rows

Scan rows record the scanner commit, fixture revisions, scanned executable
lines, wall time, peak resident memory, completeness and skipped-file count.
The two current baselines use a plain copy without nested `.git` directories.

| date | machine | spring-framework | angular | scanned source lines | wall clock | peak RSS | incomplete | skipped files |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-13 (nsd@bce31e41f88aa9b6021ad74e80a515472d95ebb4) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 583701 | 3.37s | 1821.20 MB | true | 7715 |
| 2026-10-13 (nsd@3b2c40e29155521fa422f3cb46b940e46dcc9b02 — second independent run, same plain-tree fixture) | Darwin 25.6.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 583701 | 3.32s | 1821.91 MB | true | 7715 |

## Check rows

Check rows record cold and warm wall time and peak memory, cache size after
the warm run, and the check's exit status. Cold and warm JSON must be
byte-identical. The recorded exit `2` below is an analysis result from an
included oversized base file, not a measurement-script failure.

| date | machine | spring-framework | angular | candidate | cold wall clock | cold peak RSS | warm wall clock | warm peak RSS | cache size | exit status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-15 (nsd@4a0cd37d69595466692aee1354bcdf77f8992dc2) | Darwin 27.0.0 arm64 | spring-framework@e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9 | angular@a783c4e7b753929ababa610e305112b82aaa0eb0 | 1 new .java file | 3.37s | 930.08 MB | 1.68s | 867.88 MB | 187.99 MB | 2 |

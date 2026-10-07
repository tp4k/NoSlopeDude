# Report format (`report.json` / `report.html` / terminal summary)

WS-5 aggregates every earlier stage's output into one `Report` (D17); the
terminal summary, `report.json` and `report.html` are three renderings of
that single aggregate, so the three cannot drift from each other. This
document is the JSON shape and the HTML sections; field meanings not
repeated here are documented at their owning stage: `docs/cc-rules.md` (CC,
SLOC, mass, erosion), `docs/clone-detection.md` (clone groups), and
`docs/wasteful-rules.md` (rule ids, verbosity).

## `report.json` shape

`report.json` is the canonical scan document (M6-1/M6-2), written by the same
canonical writer as `nsd check --format json` (`format::canonical_document_pretty`:
keys sorted at every level, `-0.0` written as `0.0`; pretty-printed, ended by
a newline). Nothing records time, no field holds an absolute path, and no
field holds a source excerpt: `excerpt` is in the HTML only. A callable name
is published as is only when it is non-empty and made of letters, digits and
`_ $ . #`, or is `<anonymous>@<digits>`; every other name (a computed-member
assignment is named by its whole target expression, which can hold comments,
patterns or arbitrary source) is published as `<computed>@<line>`, the
callable's start line. The rule covers scan `callables` and `top25` and check
`entities[].name` and `diagnostics[].callable`; the HTML and the terminal keep
the raw name.

```json
{
  "schema_version": 1,
  "result_scope": "scan",
  "snapshots": { "scan": "blake3:<32 hex>", "unavailable_reason": null },
  "fingerprints": { "configuration": "blake3:<32 hex>", "measurement": "blake3:<32 hex>" },
  "skipped": { "dependency_or_build_output": 0, "generated_code": 0, "gitignore": 0, "parse_grammar_setup": 0, "parse_syntax_error": 0, "parse_unreadable": 0, "parse_unsupported_extension": 0, "test": 0, "unreadable": 0, "user_exclude": 0 },
  "callables": [
    { "path": "src/Sample.java", "name": "run", "start_line": 3, "end_line": 9, "cc": 2, "sloc": 6, "mass": 4.47213595499958 }
  ],
  "scan": {
    "target": null,
    "revision": {
      "sha": "<git HEAD sha, or null>",
      "dirty": null,
      "unavailable_reason": "not_a_git_repository"
    },
    "include_tests": false,
    "exclude": ["<every --exclude glob given, in order>"],
    "min_clone_lines": 10
  },
  "scores": {
    "overall": { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "unanalyzed_lines": 0, "complete": true, "ratio": 0.0 } },
    "java":    { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "unanalyzed_lines": 0, "complete": true, "ratio": 0.0 } },
    "js_ts":   { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "unanalyzed_lines": 0, "complete": true, "ratio": 0.0 } }
  },
  "findings": [
    {
      "rule_id": "JAVA-EMPTY-CATCH",
      "language": "java",
      "location": {
        "relative_path": "src/Sample.java",
        "start_line": 11,
        "end_line": 14,
        "link": "src/Sample.java#L11-L14",
        "is_remote_link": false
      },
      "flagged_lines": [13]
    }
  ],
  "duplicates": [
    {
      "language": "js_ts",
      "redundant_lines": 10,
      "locations": [ { "...": "same SourceLocation shape as a finding" } ]
    }
  ],
  "top25": [
    {
      "name": "orderSummary",
      "language": "js_ts",
      "cc": 2,
      "sloc": 10,
      "mass": 6.324555320336759,
      "location": { "...": "see \"Top-25 span\" below" }
    }
  ],
  "skipped_files": [
    { "relative_path": "src/Broken.java", "reason": "parse_syntax_error", "detail": "salvaged; first error at line 2",
      "gaps": [{ "start_line": 2, "end_line": 2 }], "unmeasured_callables": 1 }
  ],
  "incomplete": true,
  "adaptation": {
    "summary": "CC, SLOC, mass, erosion and verbosity are computed from this scanner's own Java/JS-TS AST rules, adapted from scb-check's Python-only formulas. No numerical equivalence to scb-check's own scores is claimed.",
    "cc_rules_doc": "docs/cc-rules.md",
    "wasteful_rules_doc": "docs/wasteful-rules.md"
  }
}
```

### `scores` (D20)

Every score is reported three times: `overall`, `java`, `js_ts`. `erosion`
is `metrics::erosion` (WS-2) applied to, respectively, every callable, only
the Java callables, and only the JS/TS callables — the same published
formula, not a second implementation, since `MetricsResult.erosion` itself
is overall-only. An erosion or verbosity ratio that is exactly zero is
always rendered `0.0`, never `-0.0` (an `Iterator::sum` over an empty `f64`
slice on this toolchain is negative zero; `-0.0 == 0.0` numerically, so this
is a display normalization, not a change to any nonzero score).

### `revision` (D6)

`sha`/`dirty` are `null` and `unavailable_reason` is `"not_a_git_repository"`
for a local target that is not inside a git work tree. For a local git work
tree, `sha` is populated from `git rev-parse HEAD`, `dirty` is `null` and
`unavailable_reason` is `null`. `dirty` is `null` because no `git` command
that hashes the working tree runs on a scanned checkout: a
repository-configured filter driver would execute. For a remote
(GitHub URL) target, `sha` is the shallow clone's `HEAD`, `dirty` is always
`false` (a fresh clone), and `unavailable_reason` is `null`.

### `location.link` and `location.is_remote_link` (D6)

- Local target: `is_remote_link: false`, `link` is the repo-relative label
  `"<relative_path>#L<start_line>-L<end_line>"`.
- Remote target with a resolved sha: `is_remote_link: true`, `link` is
  `"https://github.com/<owner>/<repo>/blob/<sha>/<relative_path>#L<start_line>-L<end_line>"`.

### The canonical keys (M6-1/M6-2)

- `schema_version` is `1` and `result_scope` is `"scan"` (`"check"` for
  `nsd check --format json`).
- `scan.target` is `null` for a local target: the target as typed can be an
  absolute path, and the report names no absolute path. For a remote target
  it is the URL. The terminal summary and the HTML still show the target as
  typed.
- `snapshots.scan` is the worktree snapshot ID (`blake3:<32 hex>`) over the
  tracked entries plus the untracked entries git does not ignore
  (`.gitignore` files, `.git/info/exclude`, `core.excludesFile`; an ignored
  directory is never read, and a tracked file stays counted even when an
  ignore pattern matches it) when the target is a git work-tree root. It
  equals the ID `check` reports for a worktree candidate only when the
  checkout holds no ignored untracked entry; otherwise they differ. For any
  other target it is `null` with `snapshots.unavailable_reason`: the
  `scan.revision` reason when there is one (`not_a_git_repository` for a plain
  directory; a remote target's clone root is a work-tree root, so it carries
  its worktree snapshot ID), `target_not_git_root` (a subtree of a checkout: a worktree ID
  covers the whole checkout, not the scanned subtree) or
  `worktree_snapshot_failed`.
- `fingerprints.measurement` is the measurement-profile digest for the run's
  `--min-clone-lines`; `fingerprints.configuration` digests `--include-tests`,
  the `--exclude` globs in order and `--min-clone-lines` (family
  `nsd-scan-config-v1`). Neither holds a path.
- `skipped` counts `skipped_files` by reason, every one of the ten possible
  reasons present, zeros included. `skipped_files` stays the per-file list.
- `callables` is every measured callable, uncapped (`top25` stays the capped
  ranking), sorted by path (as a string), `start_line`, `end_line`, `name`,
  `cc`, `sloc`, then `mass`, so two runs agree whatever the thread count.
  Paths are repository-relative.

The report is byte-identical across repeated runs, checkout roots and
`RAYON_NUM_THREADS` (`tests/scan_json_identity.rs`).

### `skipped_files.reason`

One of `SkipReason`'s labels (`gitignore`, `dependency_or_build_output`,
`generated_code`, `test`, `user_exclude`, `unreadable` — a discovery-time
skip, D16) or `"parse_<ParseFailureReason label>"` (a parse failure, D18,
e.g. `parse_syntax_error`) — the `parse_` prefix is how the two kinds of
skip are told apart in one merged list.

A `parse_syntax_error` row is the one exception to "contributes nothing":
the file salvage-parses (WS-6), so only its damaged entities are excluded
and the rest is scored. It is listed so that the file behind
`incomplete: true` is named, and its `detail` is always `salvaged`,
followed by `; first error at line N` (the 1-based line of the first
`ERROR`/`MISSING` node) when tree-sitter reports one.

Each such row also carries the file's mapped parser gaps (M1-8):

- `gaps` — the 1-based, inclusive `{start_line, end_line}` line ranges of the
  file's damage spans, sorted ascending and de-duplicated. Adjacent ranges are
  not merged.
- `unmeasured_callables` — how many callables were excluded from measurement
  because they intersect the damage (fail-closed).

Both keys appear only on `parse_syntax_error` rows; every other row keeps
`relative_path`, `reason` and `detail` alone.

### `scores.<family>.verbosity` coverage (M1-8)

`scanned_lines` keeps its name but counts **analyzed** executable lines: the
distinct executable lines that survive damage pruning. The ratio is
`flagged_lines / scanned_lines`, so lines inside a pruned damage span are in
neither the numerator nor the denominator.

- `unanalyzed_lines` — the distinct executable lines of the file that damage
  pruning removed (pre-prune distinct lines minus surviving distinct lines),
  summed over the family's files. A line is in exactly one of the analyzed and
  unanalyzed sets.
- `complete` — `true` only when `unanalyzed_lines` is `0` **and** no file of
  that family failed to parse. `overall` is `complete` only when no file at all
  failed to parse. A file that fails to parse at all contributes no lines, so
  it makes its family incomplete through this flag rather than through
  `unanalyzed_lines`.
- An empty family has `scanned_lines` 0 and `ratio` `0.0`.

### Top-25 span

`Callable` (D8) publishes both `start_line` and `end_line`
(`IrCallable::span.end_line`, M0c-13). A top-25 row's `location` is a
real `[start_line, end_line]` region — the callable's whole declaration
and body, not just its first line — and the HTML `excerpt` fully brackets it
(`report.json` carries none), read back off disk, the same as every
finding's and every duplicate-group location's span. A single-line callable (a one-line arrow function, e.g.
an expression-bodied `(b) => b`) still has `start_line == end_line`, same
as any other one-line region.

## `report.html` sections

One self-contained file (D19): inline `<style>`, no template engine, no
CDN or external asset, and no scheme prefix anywhere except a legitimate
`<a href="https://github.com/...">` blob link for a remote target. Every
value interpolated from the scanned repository — a source excerpt, a file
path, a rule id, a callable name — passes through the module's single
`escape_html` helper.

Sections, in document order, each its own `<section id="...">`:

1. **`#scan-settings`** — target, revision (sha/dirty/unavailable reason),
   `include_tests`, every `exclude` glob, `min_clone_lines`, and the
   `incomplete` marker (`#incomplete`).
2. **`#scores`** — one table row per family (`overall`/`java`/`js_ts`);
   each numeric cell carries a stable `id` (`#erosion-<family>`,
   `#verbosity-<family>`) so a reader — or a test — can cross-check a
   number against `report.json` without parsing prose.
3. **`#findings`** (`#findings-count`) — one row per rule finding: rule id,
   language, and its location (link + excerpt).
4. **`#duplicates`** (`#duplicates-count`) — one block per clone group:
   language, redundant line count, and every occurrence's location.
5. **`#top25`** (`#top25-count`) — one row per top-25 callable: name,
   language, CC, SLOC, mass, and its (single-line, see above) location.
6. **`#skipped-files`** (`#skipped-count`) — one row per skipped file:
   path, reason, detail.
7. **`#adaptation`** — the same adaptation summary and doc links as the
   JSON, so a reader of the HTML alone sees the non-equivalence disclosure
   the Assumptions section requires.

## Check JSON (M6-1/M6-2 subset)

`nsd check --format json` prints one compact JSON document followed by `\n`.
It is the subset of M6-1/M6-2 that `check` needs now; fields may be added by
M6-2, and these are not meant to be renamed. Keys are sorted at every level,
every number is an integer except the `ratio` and `erosion` floats, paths
are repository-relative (`%XX` for non-UTF-8
bytes), nothing records time, and no field except a pass-through `message`
(see below) records the checkout's location.

```json
{"diagnostics":[...],"exit_status":3,"result_scope":"check","schema_version":1}
```

- `schema_version` is `1`, `result_scope` is `"check"`, and `exit_status` is
  the process exit code (`0`-`3`).
- `diagnostics` keeps the order of the terminal listing, one entry per line.
  Each entry has `code` and the fields of its terminal line:

| Codes | Fields besides `code` |
|---|---|
| `NSD-E101`, `NSD-E102`, `NSD-G102` | `path`, `start_line`, `end_line`, `callable`, `base` (`{path, start_line, end_line, cc, sloc}` or `null`) |
| `NSD-V101` | `path`, `start_line`, `end_line`, `rule_id` |
| `NSD-S101`, `NSD-S102` | `path`, `directive_line`, `rule_id` (a string or `null`) |
| `NSD-A101` | `path`, `start_line`, `end_line` |
| `NSD-A102` | `path`, `reason` |
| `NSD-V102` | `path`, `start_line`, `end_line`, `added_lines`, `matched` (`{path, start_line, end_line}`) |
| `NSD-C101`, `NSD-C102` about the candidate config | `path` (`"nsd.yml"`) |
| `NSD-G101`, `NSD-C102` for a refused `--config` | `message` |

`scan`'s `report.json` uses the same canonical writer and the same
`snapshots`, `fingerprints` and `skipped` keys; see "The canonical keys".

### The canonical check document (M6-2)

A check that ran to a verdict adds these top-level keys (still sorted, still
compact):

- `snapshots`: `{base, candidate}`, each a snapshot ID (below).
- `fingerprints`: `{configuration, measurement}`, each `blake3:` plus
  lowercase hex. The configuration fingerprint covers the trusted config's
  version, `include` (omitted differs from listed), `exclude`,
  `measurement.min_clone_lines` and the five severities; output caps are
  excluded. The measurement fingerprint uses the trusted `min_clone_lines`.
- `skipped`: per-reason counts, always all 13 keys (zero when none),
  including `parse_syntax_error`.
- `entities`: every changed callable (`path`, `name`, `start_line`,
  `end_line`, `cc`, `sloc`, and `base` (`{path, start_line, end_line, cc,
  sloc}`) or `null`), sorted by path, start line, name.
- `summaries`: `scope`, `files`, and `overall`, `java`, `js_ts`, each
  `{erosion, verbosity: {ratio, flagged_lines, scanned_lines,
  unanalyzed_lines, complete}}`. `flagged_lines` counts rule findings only;
  clone-group lines are not in it. `scope` is `"changed"` while trusted V102
  is off (unchanged files are not read) and `"full"` while it is on
  (unchanged files contribute their cached facts); `files` counts the files
  summarized.
- `coverage`: one entry per changed file, sorted by path (`path`, `analyzed_lines`,
  `unanalyzed_lines`, `complete`, `gaps`), each gap
  `{base: {start_line, end_line} or null, candidate: {...}, tolerated}`.

A failure document (a check that stopped before both sides exist, so
`details` is absent) carries only the four base keys: `diagnostics`,
`exit_status`, `result_scope`, `schema_version`.

Numbers: counts and lines are integers. `ratio` and `erosion` are floats in the shortest round-trip form; `-0.0` is written `0.0`; non-finite
values do not occur.

Snapshot IDs: `blake3:` plus 32 lowercase hex digits. The digits are the
little-endian `u128` of the first 16 BLAKE3 bytes, printed big-endian, so
they are the byte-reverse of the raw hash's hex. Decision (ledger rows 57,
60): 128 bits are kept; the birthday bound is about 2^64 snapshots, and the
ID labels trusted local content rather than authenticating it.

Free-text `message`s of `NSD-G101` and `NSD-C102` are path-free in JSON: the
checkout, its git directories and the refused config path are replaced; the
terminal listing still prints the original text. The document is
byte-identical across checkout roots and thread counts.

`--worktree` always computes the worktree snapshot ID, so it reads the
changed files once more; a tracked entry under a non-directory or symlinked
parent fails `NSD-G101` (ledger row 59; the check-then-read race remains).

## Terminal summary

Printed to stdout by `nsd::report::terminal_summary`: the scan
settings, the three families' erosion and verbosity ratio, the count of
findings/duplicate groups/top-25 rows/skipped files, the `incomplete`
marker, and the adaptation note — the same numbers as the two files, in
prose form.

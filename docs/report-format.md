# Report format (`report.json` / `report.html` / terminal summary)

WS-5 aggregates every earlier stage's output into one `Report` (D17); the
terminal summary, `report.json` and `report.html` are three renderings of
that single aggregate, so the three cannot drift from each other. This
document is the JSON shape and the HTML sections; field meanings not
repeated here are documented at their owning stage: `docs/cc-rules.md` (CC,
SLOC, mass, erosion), `docs/clone-detection.md` (clone groups), and
`docs/wasteful-rules.md` (rule ids, verbosity).

## `report.json` shape

```json
{
  "scan": {
    "target": "<the CLI target argument, verbatim>",
    "revision": {
      "sha": "<git HEAD sha, or null>",
      "dirty": true,
      "unavailable_reason": "not_a_git_repository"
    },
    "include_tests": false,
    "exclude": ["<every --exclude glob given, in order>"],
    "min_clone_lines": 10
  },
  "scores": {
    "overall": { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "ratio": 0.0 } },
    "java":    { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "ratio": 0.0 } },
    "js_ts":   { "erosion": 0.0, "verbosity": { "flagged_lines": 0, "scanned_lines": 0, "ratio": 0.0 } }
  },
  "findings": [
    {
      "rule_id": "JAVA-EMPTY-CATCH",
      "language": "java",
      "location": {
        "relative_path": "src/Sample.java",
        "start_line": 11,
        "end_line": 14,
        "excerpt": "<the lines [start_line, end_line], read back off disk>",
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
    { "relative_path": "src/Broken.java", "reason": "parse_syntax_error", "detail": "..." }
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
tree, `sha` and `dirty` are populated from `git rev-parse HEAD` /
`git status --porcelain` and `unavailable_reason` is `null`. For a remote
(GitHub URL) target, `sha` is the shallow clone's `HEAD`, `dirty` is always
`false` (a fresh clone), and `unavailable_reason` is `null`.

### `location.link` and `location.is_remote_link` (D6)

- Local target: `is_remote_link: false`, `link` is the repo-relative label
  `"<relative_path>#L<start_line>-L<end_line>"`.
- Remote target with a resolved sha: `is_remote_link: true`, `link` is
  `"https://github.com/<owner>/<repo>/blob/<sha>/<relative_path>#L<start_line>-L<end_line>"`.

### `skipped_files.reason`

One of `SkipReason`'s labels (`gitignore`, `dependency_or_build_output`,
`generated_code`, `test`, `user_exclude`, `unreadable` — a discovery-time
skip, D16) or `"parse_<ParseFailureReason label>"` (a parse failure, D18,
e.g. `parse_syntax_error`) — the `parse_` prefix is how the two kinds of
skip are told apart in one merged list.

### Top-25 span (a known compromise)

`Callable` (D8, owned by WS-2) publishes only `start_line`: the callable's
own declaration line, not its body's end line. A top-25 row's
`location.start_line == location.end_line`, and `excerpt` is that one line
read back off disk — not the callable's whole body. Every finding's and
every duplicate-group location's span, by contrast, is a real
`[start_line, end_line]` region and its excerpt fully brackets the flagged
code. See the WS-5 implementer report for why this was not solved by
re-parsing or by hand-rolled brace-matching (both are out of this stream's
scope).

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

## Terminal summary

Printed to stdout by `agent_slope::report::terminal_summary`: the scan
settings, the three families' erosion and verbosity ratio, the count of
findings/duplicate groups/top-25 rows/skipped files, the `incomplete`
marker, and the adaptation note — the same numbers as the two files, in
prose form.

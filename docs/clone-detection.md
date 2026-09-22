# Duplicate-block detection (clones)

Clone detection is grammar-derived, like CC and SLOC (see `docs/cc-rules.md`):
candidate blocks come from the pinned Tree-sitter parse trees, never from a
textual/regex line matcher.

## Candidate blocks (D15)

A candidate is any contiguous run of **two or more** sibling statements taken
from one of these containers:

- **Java** — the statements of a `block`, of a `constructor_body`, or of a
  `switch_block_statement_group` (its `switch_label` children are not
  statements and are excluded). `switch_block` itself is not a candidate
  container: its own children are `switch_block_statement_group`/`switch_rule`
  nodes, not statements. `program` is not a candidate container either: in
  Java it holds type declarations, not statements.
- **JS/TS** — the statements of a `statement_block`, the top-level statements
  of a `program` (a file's own module-level statements, outside any
  function), or the `body` field of a `switch_case`/`switch_default` (there
  is no wrapper node around a switch arm's statements, so the `body` field is
  read directly).

Every contiguous sub-run of length ≥ 2 inside one of these containers is its
own candidate — including the full container, which is simply the run of
maximal length. A single repeated statement is never a candidate, however
long: it takes two statements at minimum to be considered duplication.

A candidate only qualifies if its D11 source-line count (below) is at least
`--min-clone-lines` (default 10, `DEFAULT_MIN_CLONE_LINES`).

## Source lines within a candidate (D11, reused)

A candidate's size is measured with the same per-line rule used for a
callable's SLOC: the count of distinct 1-based source lines, within the
candidate's statements, that contain at least one leaf token belonging to a
named, non-comment node. A bare `break;`/`continue;`/`return;` (no expression)
still counts as its own line, per the same documented exception as D11.
Measuring duplicate size this way — not a raw `end_line - start_line + 1`
span — means two occurrences that differ only in comments or blank lines
still measure to the same size and stay comparable.

## Normalization and grouping (D14)

Each candidate is reduced to a normalized token stream: the language family's
prefix (`java` or `js_ts` — see D20), followed by one token per leaf
descendant of its statements, in order:

- An **anonymous** leaf (punctuation, keywords) contributes its `node.kind()`.
- A **named** leaf (identifiers, literals) contributes its exact source text.
- Comment nodes are skipped entirely.

Candidates are grouped by a **128-bit fingerprint of that exact normalized
token stream**, not by the stream text itself: the byte content the
fingerprint is computed over is unchanged from the paragraph above, only the
grouping key's representation is a fingerprint rather than the stream's own
bytes, so the retained memory per candidate does not grow with its token
count. The collision probability at these candidate volumes is a
non-concern in practice — the same principle content-addressed systems rely
on. Two candidates group together only when this fingerprint is identical.
Because the underlying stream preserves every identifier and literal
verbatim, renaming one variable or changing one literal is enough to put a
block in a different group (or no group at all, if nothing else duplicates
it) — normalization only removes formatting, whitespace and comments.
Because the stream is prefixed with the language family, a Java block and a
JS/TS block are never grouped even if their token text happens to match
exactly.

A group needs at least two candidates (from anywhere in the scanned tree,
including two spots in the same file) to be reported at all.

## Maximal filter (D15)

Because every contiguous sub-run is its own candidate, a single duplicated
container produces one candidate per sub-run length, and (naively) one
"group" per length. Reporting all of them would be redundant: if a 5-statement
block duplicates in two files, its 4-statement, 3-statement and 2-statement
sub-runs duplicate right along with it. The maximal filter removes this
redundancy: a group is dropped if every one of its occurrences lies inside
the corresponding occurrence of some other, still-surviving, larger reported
group in the same file (same `relative_path`, and the smaller occurrence's
line span falls entirely within the larger one's). Groups are considered
largest (by statement count) first, so only genuinely independent duplication
is left standing.

## Ranking (D14) and `redundant_occurrences`

Within a group, occurrences are sorted into a canonical order by
`(relative_path, start_line)`; the first in that order is the group's "first
occurrence" — the one occurrence that isn't itself redundant. A group's
ranking metric, `redundant_lines`, is the sum of the D11 source-line count of
every *other* occurrence: the lines that would disappear if every other
occurrence were replaced with a call to the first. Reported groups are sorted
by `redundant_lines` descending, ties broken by the first occurrence's
`(relative_path, start_line)`.

`redundant_occurrences(group)` returns exactly the slice of occurrences that
`redundant_lines` was summed over (every location but the canonically first)
— the single shared definition of "beyond first occurrence" that later
stages (verbosity scoring) reuse rather than recomputing.

### Worked example

Two files each define `run()` with the same four statements
(`alpha(...)`/`beta(...)`/`gamma(...)`/`delta(...)`, one argument per line),
12 D11 source lines total per occurrence:

```
group   = { DupA.java:run (12 lines), DupB.java:run (12 lines) }
first   = DupA.java (alphabetically first path)
redundant_occurrences = [ DupB.java:run ]
redundant_lines        = 12
```

If a third, five-occurrence group of 10-line blocks exists elsewhere in the
same scan, it outranks the two-occurrence, 12-line group above:
`(5 - 1) * 10 = 40 > 12`, even though each of its individual occurrences is
smaller — redundant lines are summed over every occurrence past the first,
not compared per-occurrence.

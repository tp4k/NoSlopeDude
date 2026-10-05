# nsd — structural regression gate for agent-heavy codebases (v1, frozen)

Supersedes `nsd-plan.md` and `nsd-plan-reviewed.md`. Where either disagrees
with this document, this document wins. Section *Corrections applied* records
what changed and the evidence, so no correction has to be rediscovered.

---

## Context

`~/pet/agent_slope` is a working 3,269-line Rust CLI that measures structural
quality at **one revision**: per-callable cyclomatic complexity and executable
SLOC from tree-sitter, `mass = CC × √SLOC`, `erosion = Σmass(CC>10) / Σmass`,
AST clone detection, six wasteful-code rules, and a verbosity score. Its own
`initial_plan.md` states the limitation plainly: *"This is a snapshot tool;
commit history and private remote authentication are deferred."*

That limitation is the whole problem. A single erosion number says a codebase
is in trouble but not *who* made it worse or *when*. It cannot gate anything:
you cannot fail a commit for a number the repo already had before that commit
existed.

`nsd` (NoSlopDude) is v1 of the gate — the same measurement engine plus a git
layer, so the tool answers **"did this change make it worse?"** instead of
"how bad is it?". The outcome is a pre-commit hook and a CI check that block
one class of regression — a callable crossing into high complexity, a newly
duplicated block, a new rule finding — attributed to the change that
introduced it, with policy read from the *base* snapshot so a candidate cannot
weaken its own gate.

Source spec: `~/Downloads/LSD_plan_openai_v2.md` (3,068 lines, 90 sections).
This plan supersedes it where they differ; differences are under *Spec deltas*.

### Measured starting state

Three recorded scans, with the caveats that decide which may serve as a golden:

| fixture | language | authorship | sha | erosion | `dirty` | `incomplete` | usable as golden |
|---|---|---|---|---|---|---|---|
| `java-fixture-01` | Java, with a small JS/TS component | unknown | `c6671504394b7c862dac032bc7c7364ad3af8b7f` | `0.13313797553867948` (java `0.13294913151043064`) | false | **false** | **yes — the only one** |
| `js-ts-fixture-01` | JS/TS | unknown | `1c61294688f50bc8770dc251b96d528e905fe0ce` | `0.26197032832368317` | false | true | no — incomplete |
| `js-ts-fixture-02` | JS/TS | unknown | `257b564afb4baf6ef2774404e038a3350b385fb2` | `0.5398929561208313` | **true** | true | no — dirty *and* incomplete |

Private repository names are omitted. Authorship must be recorded as human,
AI, mixed, or unknown; the archived reports establish language and measurements,
not authorship, so these entries remain unknown until verified. Resolve local
fixture locations from the archived reports in `~/pet/agent_slope` by matching
`scan.revision.sha` to the pins above. Keep checkout paths, source excerpts,
and private repository-name mappings out of the new repository.

`java-fixture-01` is the Java-bearing fixture (215,924 Java scanned lines).
`js-ts-fixture-01` and `js-ts-fixture-02` are **JS/TS-only** — both report `java.verbosity.scanned_lines
= 0` and `java.erosion = 0.0`.

---

## Corrections applied

Errors carried by `nsd-plan.md` (P) or introduced by `nsd-plan-reviewed.md` (R),
each verified rather than asserted.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | R: *"Orchard `0.5.18` does not exist"*; pin `0.5.15` | **R wrong — P right** | crates.io `max_version = 0.5.18`, published 2026-09-18; `docs.rs/.../latest` serves 0.5.18 (HTTP 200). A docs.rs `/latest/` URL shows the newest version whose *docs built*, and cannot prove non-existence. |
| 2 | P+R: orchard *forces* `tree-sitter` 0.25.10 → 0.27.0 | **both wrong** | Orchard 0.5.18 runtime deps are `tree-sitter-language ^0.1` + `cc`; `tree-sitter ^0.26.12` is **dev-only**. A probe crate with `tree-sitter 0.25.10` + orchard 0.5.18 calling `set_language` compiles clean (`cargo check` exit 0). The bump is **elective** — see D-GRAMMAR. |
| 3 | P: `js-ts-fixture-01` is Java-bearing | **wrong** | `the archived JS/TS fixture 01 report`: `java.scanned_lines = 0`. Its orchard delta is necessarily zero. |
| 4 | P: erosion `0.5399` as reproducible baseline | **wrong** | `the archived JS/TS fixture 02 report`: `revision.dirty = true`, and `incomplete = true`. |
| 5 | P: "9 suites" | **wrong — 10** | `tests/` holds ten `.rs` suites; P's own inline list names ten. |
| 6 | P: "§12 M12" | **wrong — §84** | `M12 — Cache + Large-Repo Performance` at `LSD_plan_openai_v2.md:2880`, under `# 84. MVP Milestones`. |
| 7 | R: `java-fixture-01` validates **WebJar** exclusion | **unsupported** | Zero webjar path mentions in its 854 skips (853 `test`, 1 `dependency_or_build_output`). Criterion would pass vacuously; dropped. |
| 8 | R: persisted `DefaultHasher` output | **R right, and load-bearing** | `src/clones/mod.rs:427-434` builds the 128-bit clone fingerprint from two `DefaultHasher` (SipHash) streams. Harmless in-memory today; becomes a persistent cache key at M5, and its output is not stable across Rust releases. |
| 9 | R: suppression example `nsd-ignore[NSD-V101]` → a rule ID | **R right** | `NSD-V101` is a diagnostic code, not a rule. `JSTS-EMPTY-CATCH` is real (`src/rules/mod.rs:31`). |
| 10 | R: `NSD-C101` fires on any config change, exit table maps config → `2` | **contradiction, fixed** | Would exit 2 on every legitimate config edit. C101 is now informational. |
| 11 | R: base config may set `warn`, exit table has no warn state | **gap, fixed** | `warn` now defined as exit-neutral. |
| 12 | R: tests "unused suppressions", no such diagnostic defined | **gap, fixed** | `NSD-S102` added. |
| 13 | R: ordinal removed from the finding fingerprint, no replacement | **gap, fixed** | Order-preserving greedy pairing specified under *Finding matching*. |
| 14 | P: spec is "3,069 lines" | **3,068** | `wc -l`. |
| 15 | F: CLI grammar block lists neither `--config` nor `--allow-new-suppressions` | **internal gap, fixed** | The prose under *CLI and configuration* relies on both ("a trusted external `--config <path>` overrides it"; "unless the invocation supplies `--allow-new-suppressions`"), but the fenced grammar shows only `--format`. Grammar block extended. |
| 16 | F: `E101`/`E102`/`V102`'s "only escapes are the invocation flag and an explicit `off`" | **internal contradiction, fixed** | `--allow-new-suppressions` affects `S101` alone. Trusted policy may downgrade these diagnostics to exit-neutral `warn` or disable them with `off`; neither requires a source suppression. |
| 17 | F: "`blake3` is the one new runtime dependency v1 takes on" | **wrong — three** | *Settled decisions* chooses `git2` (libgit2) for index reads, merge-base and blob OIDs, and `nsd.yml` needs a YAML parser (`serde_yaml_ng`; `serde_yaml` is deprecated). All three are new runtime dependencies. |
| 18 | F: freeze fingerprint includes "all four grammar versions" | **miscount, fixed** | There are three grammars (`tree-sitter-java-orchard`, `tree-sitter-typescript`, `tree-sitter-javascript`). The fourth version is the `tree-sitter` runtime, which belongs in the fingerprint but is not a grammar. Wording fixed here and in M0c step 12. |

Final-review corrections also make M0b compare both binaries against the same
fixed fixture checkout with identical revision metadata and legacy serialization,
and carry the malformed-fixture exception into the acceptance criteria below.

F = this document, found by the 2026-09-22 implementation review. The
implementation plan's draft claimed *five* such inconsistencies; four were
recoverable and are recorded above, the fifth could not be reconstructed and
is not claimed.

### Amendments (2026-09-22, from the implementation grilling)

Decisions taken after the freeze, applied in place below so this document
stays the single spec. Each is a deliberate change, not a correction.

| # | Amendment | Where applied |
|---|---|---|
| A1 | **Golden is a digest.** The committed reference for `java-fixture-01@c6671504…` is a numbers-and-hashes digest (per-language erosion and verbosity, findings per rule ID, clone group count and redundant lines, skips per reason, top-25 as `(cc, sloc, mass)` triples, and a BLAKE3 of the excerpt-stripped, path-relativised report body). The full `report.json` carries private source excerpts and an absolute checkout path and stays in `~/pet/agent_slope`; the byte-identical M0b gate runs locally against it, from the same checkout path the archived scan recorded. | *Settled decisions* → Golden fixture; M0b step 8; *Test and acceptance plan* |
| A2 | **`measurement.min_clone_lines` is an `nsd.yml` key.** Any positive integer, default `10`, read from trusted base config for `check` and from the repository-root `nsd.yml` for `scan` (the `--min-clone-lines` flag overrides it). It enters the measurement fingerprint and the cache key. Supersedes "existing flags become config keys for `check` and stay flags for `scan`". | *CLI and configuration*; *Modified* → `src/cli.rs` |
| A3 | **`V102` maps moves before it fires.** A clone occurrence deleted from the base maps to an added candidate occurrence when the base occurrence's normalized token stream appears as one contiguous run inside the added stream and the added occurrence's extra executable lines are within `max(1, floor(base_occurrence_lines / 10))`. Mapped pairs are moves and never raise `V102`; an unmapped added occurrence with a qualifying match elsewhere does. Repeated identical occurrences pair *k*-th to *k*-th in source order, as findings do. | *Diagnostics* → `NSD-V102`; M3–M5 |
| A4 | **MSRV is declared, not exercised.** `rust-version = "1.90"` in `Cargo.toml` (forced by `tree-sitter` 0.27.0); no test leg compiles on 1.90. The local toolchain is Homebrew Rust, one version at a time. | *Settled decisions* → D-GRAMMAR |
| A5 | **Provenance pin.** The import source is `agent_slope@912ec7a1ca2b9e5dd1ffa976a51f98d7d00d6704` (branch `docs/close-scanner-run-ledger`), which tracks `docs/deferred-work.md` and both perf rows in `docs/measurements.md`. `c50d6bd` lacks both. | new-repo README |
| A6 | **Rename boundary tests.** libgit2 scores similarity on hashed content chunks, so exact 49 %/50 % fixtures are not reliably constructible. The suite asserts the configured threshold (`rename_threshold = 50`) plus one clear rename and one clear non-rename fixture. | M1–M2 tests |
| A7 | **Orchard hypothesis verified on the one-line shape.** A probe crate pinning `tree-sitter-java-orchard` 0.5.18 + `tree-sitter` 0.27.0 + `tree-sitter-typescript` 0.23.2 + `tree-sitter-javascript` 0.25.0 compiles, and parses `void f(Class<?> @Nullable ... cs) {}` without error (`spread_parameter` gains an `annotations:` field). The TS `using` and JSX bare-`&` shapes still fail as recorded. The 55-file corpus run in M0c remains the gate. | M0c step 10 |

Final-review amendments, approved after A1–A7:

| # | Amendment | Where applied |
|---|---|---|
| A8 | **Private fixture labels.** Hide private repository names; identify each fixture by language and verified human/AI/mixed authorship, or unknown when unverified. Keep private source and checkout mappings local. | *Measured starting state*; import instructions |
| A9 | **Suppressions bind to findings.** Match a suppression by rule ID and its matched underlying finding. Moving that finding together with its unchanged directive is tolerated; reusing the directive for an unmatched finding raises `S101`. | *Stable data model*; M3–M5; acceptance tests |
| A10 | **Clone-index completeness includes unchanged files.** When `V102` is enabled (`deny` or `warn`), an included unchanged file required for clone comparison that cannot be analyzed because of size, encoding, or capability failure raises `A102`. Trusted exclusion removes it from scope; mapped legacy parse-damage tolerance is unchanged. | *Required clone coverage*; M3–M5; acceptance tests |

Post-freeze amendments (2026-10-05, user decision):

| # | Amendment | Where applied |
|---|---|---|
| A11 | **A trusted `--config` must lie outside the candidate checkout.** A `--config <path>` inside the checkout is refused with `NSD-C102`, exit `2`. It is never read and no policy evaluation runs. "Inside" holds when any of these does: (1) the absolute given path, meaning the path joined to the current directory with `.` and `..` removed lexically, lies under the repository's canonical work-tree root; (2) a canonical (symlink-resolved) form of it does: the whole path, its directory with the file name appended, or the path as the OS resolves it, so `..` after a symlink counts. When the file is missing, its longest existing prefix is resolved; (3) filesystem identity: an existing directory among the ancestors of the forms in (1) and (2) has the same `(st_dev, st_ino)` as the work-tree root. So an alias of the work-tree root itself or of one of its ancestors that canonicalization does not merge, such as the macOS firmlink `/System/Volumes/Data<root>`, a bind mount of the root or of an ancestor, or a case variant on a case-insensitive volume, still counts. Only such aliases are promised: the identity clause compares against the root's identity alone. `.git/` is included throughout. Ancestors come from those normalized and resolved forms, never from the raw path's own prefixes, so `sub/../../x.yml` that leaves the checkout is outside. A missing path is judged by (1) and (2), and by (3) through whichever ancestors exist. Two exceptions are documented and not refused: a hard link outside the checkout to a file inside it, and a bind mount of a subdirectory of the checkout (not of the root) at a path outside it. Like a copy, each is a separate name only the invoker can create. A `--config` file outside the checkout completely replaces repository policy for that invocation. | *CLI and configuration*; M3–M5; *Test and acceptance plan* |

R also deleted P's *Settled decisions*, *Architecture*, *Spec deltas* and
*Rejected during grilling* — roughly half the plan, including the `git2`
choice, the module map, and the quantified reasons each alternative lost.
All are restored here. R additionally introduced five diagnostics
(`A102`, `G101`, `G102`, `C101`, `C102`) without listing them as changes to a
frozen namespace; they are good and are adopted **explicitly** below.

---

## Settled decisions

Resolved during grilling and by the measurements above. Not open for
re-litigation at implementation time.

| Area | Resolution |
|---|---|
| Repo | New repo at `~/pet/nsd`, engine ported from `agent_slope`; `agent_slope` archived |
| Name | `nsd` (free on crates.io; `ns` taken). Config `nsd.yml`. Profile `nsd-v1` |
| Languages | Java + JS/TS/TSX/JSX only. **Python stays cut** — not for parser reasons (`tree-sitter-python` 0.25.0 would drop in today) but because no validated reference exists to check Python numbers against, and a third lowering triples conformance burden before the IR has proven itself on two. v1.1 candidate |
| **D-IR** | **tree-sitter parse → lowering → normalized IR → analyzers.** `cc`, `sloc`/D11, clones and the six rules stop reading `node.kind()` and read the IR instead. Justified by v1's own costs — grammar swaps become one-lowering changes, and cross-language comparability becomes testable — **not** by parser portability |
| Diagnostics | `NSD-` namespace, closed at the twelve codes listed below |
| Git library | `git2` (libgit2) — needed for index reads, merge-base, blob OIDs |
| Crate layout | Single crate, workspace-ready |
| Java grammar | **`tree-sitter-java-orchard` 0.5.18** (current max; 0.5.15 was a misread) |
| **D-GRAMMAR** | **`tree-sitter` 0.25.10 → 0.27.0, MSRV 1.90.** *Elective, not forced* (correction #2). Taken deliberately: 0.27.0 is current max, the alternative is pinning a runtime that drifts from the grammar ecosystem, and MSRV 1.90 is already satisfied (local toolchain 1.98.1) and is **declared only** — `rust-version = "1.90"`, no 1.90 compile leg (A4). Recorded as a choice so the cost is not rediscovered as a surprise. |
| JS/TS grammars | `tree-sitter-typescript` 0.23.2 and `tree-sitter-javascript` 0.25.0 — **both already at max published version**; 0.23.2 dates from 2024-11-11 and is effectively unmaintained. No upgrade path exists; see *Parse recovery*. |
| Parsers | **tree-sitter, for every language, full stop.** Java via orchard, JS/TS via `tree-sitter-{typescript,javascript}`. No second parser family is planned, prototyped or budgeted in v1. oxc's rejection as a *metrics source* stands permanently; as a frontend it is a v1.1 conversation nobody needs to have yet |
| Damage spans | Parsing MUST yield a partial tree with **typed damage spans** on malformed input. This is an IR invariant, not a plugin-eligibility rule: whole-file loss is the defect *Parse recovery* exists to fix, and tree-sitter's error recovery is what makes salvage possible |
| Verbosity numerator | Union of distinct executable `(path,line)` positions across all clone occurrences and all rule findings |
| Parse failure | **Salvaged, not dropped whole-file** — see *Parse recovery* |
| Callable matching | Tiers 1+2+3 (structural → git rename → body fingerprint); 4/5 deferred |
| `NSD-E102` trigger | CC increase **OR** `candidate_sloc - base_sloc > max(1, floor(base_sloc / 10))` |
| Fingerprints | BLAKE3, explicitly versioned. `DefaultHasher` output is never persisted |
| Suppressions | Preceding-line only, core rule IDs only. Blocked by default; escape is the `--allow-new-suppressions` invocation flag, never repo config |
| Config trust | Policy read from the **base** snapshot |
| Hook scope | Changed files for erosion + rules; clones via cached base index |
| CI scope | Full two-snapshot scan; repo ratios **reported, never gated** |
| Clone cache | Keyed by git blob OID + grammar + measurement fingerprint, under the common git dir |
| Test files | Covered by the *gate*; `scan` ratios keep the current default exclusion |
| Exit codes | `0` pass · `1` policy regression · `2` analysis error · `3` both |
| HTML report | Kept, **`scan` only** |
| Watch mode | Deferred to v1.1 |
| Golden fixture | **`java-fixture-01@c6671504…`** — the only clean, complete scan on record. Committed as a numbers-and-hashes **digest**; the full `report.json` (private excerpts, absolute path) stays in the `agent_slope` archive (A1) |
| Measurement profile | Fingerprint includes **IR version + per-language lowering version + the three grammar versions + the `tree-sitter` runtime version + `measurement.min_clone_lines`** → finalize grammars, land the IR, re-baseline, *then* freeze. Grammar versions stay in the fingerprint even under D-IR: a lowering can be correct and still be handed different input |

---

## Public interfaces and policy

### CLI and configuration

```
nsd scan  <target> --output <dir> [--include-tests] [--exclude ...] [--min-clone-lines <n>]
nsd check --staged      [--config <path>] [--format terminal|json|agent] [--allow-new-suppressions]
nsd check --base <ref> [--worktree] [--config <path>] [--format terminal|json|agent] [--allow-new-suppressions]
```

`--staged` and `--base` are mutually exclusive; `--worktree` requires `--base`;
`terminal` is the default format (correction #15). `--min-clone-lines` defaults
to `measurement.min_clone_lines` from the repository-root `nsd.yml` when one
exists, else `10` (amendment A2).

- `--staged`: base is `HEAD`, candidate is the Git **index**; staged paths are
  never read from the worktree (invariant #1).
- `--base`: base is `merge-base(HEAD, ref)`; candidate is `HEAD`, or the
  complete worktree overlay when `--worktree` is supplied.
- Discover only repository-root `nsd.yml`. A trusted external `--config <path>`
  overrides it; a path inside the candidate checkout is refused with `NSD-C102`
  (amendment A11).
- Checks use the **base snapshot's** config. When base has none, immutable
  built-in defaults judge the change; candidate config is validated and
  reported but applies only to later checks.
- Base config may control include/exclude scope, output caps,
  `measurement.min_clone_lines` (amendment A2), and
  `deny|warn|off` for `NSD-E101`, `E102`, `V101`, `V102` and `S102`.
  Measurement algorithms, the CC threshold, analysis-error handling and
  suppression policy are immutable; the clone threshold is the one
  configurable measurement input and enters the fingerprint (A2).
- `NSD-S101` remains denied unless the invocation supplies
  `--allow-new-suppressions`; repository config cannot enable it.

### Diagnostics

| Code | Meaning | Default | Exit contribution |
|---|---|---|---|
| `NSD-E101` | candidate callable has `CC > 10`; matched base callable absent or `CC <= 10` | deny | 1 |
| `NSD-E102` | base and candidate both `CC > 10`, **and** CC increased **or** `candidate_sloc - base_sloc > max(1, floor(base_sloc/10))` | deny | 1 |
| `NSD-V101` | unmatched new core-rule finding | deny | 1 |
| `NSD-V102` | one diagnostic per maximal clone occurrence newly introduced or materially extended across changed executable lines, with a qualifying match elsewhere; a deleted base occurrence that maps by contiguous containment within the extension threshold to an added occurrence is a move, not a regression (A3) | deny | 1 |
| `NSD-S101` | newly added suppression, including an existing directive transferred to an unmatched finding | deny (flag-only escape) | 1 |
| `NSD-S102` | invalid, unknown-rule, or unused suppression directive | warn | 0 |
| `NSD-A101` | new, unmappable, intersecting, or worsened parse damage | error | 2 |
| `NSD-A102` | required analyzer capability unavailable, invalid encoding, or included changed file above the supported size limit; also required unchanged clone input unavailable for these reasons while `V102` is enabled | error | 2 |
| `NSD-G101` | merge base or required snapshot unavailable | error | 2 |
| `NSD-G102` | callable-match ambiguity that could change an E101/E102 verdict | error | 2 |
| `NSD-C101` | candidate configuration changed | **informational** | **0** |
| `NSD-C102` | invalid base, candidate, or trusted configuration | error | 2 |

Exit resolution: `0` pass · `1` denied regressions · `2` analysis/config/
snapshot errors · `3` both. **`warn` and informational diagnostics print and
appear in JSON but never change the exit code** (closes corrections #10, #11).
`NSD-C101` exists so a config change is *visible* in review, not so it fails
CI; `NSD-C102` is what stops broken future policy merging silently.

### Stable data model

- Add normalized `RepoPath`, byte-and-line `Span`, `Callable::end_line`,
  lexical ownership, callable kind/signature, structural identity, body
  fingerprint.
- **BLAKE3**, explicitly versioned, for measurement identity, callable bodies,
  findings, suppressions and clone token streams. `DefaultHasher` output is
  never persisted (correction #8). `blake3` is one of the three new runtime
  dependencies v1 takes on — with `git2` and `serde_yaml_ng` (correction
  #17) — and it is taken to make the M5 cache sound.
- Callable matching, in order: (1) structural identity; (2) Git rename plus
  structural identity; (3) exact normalized body fingerprint. Ambiguous tier-3
  matches remain unmatched unless that could change policy classification, in
  which case emit `NSD-G102`.
- **Finding matching** uses rule ID, matched callable or lexical/file context,
  normalized syntax, renamed paths and diff-line mapping. Repeated identical
  findings within one context are paired by **order-preserving greedy matching
  in source order** — the *k*-th base occurrence pairs with the *k*-th
  candidate occurrence, and only surplus candidate occurrences raise `V101`.
  No ordinal is embedded in any persisted fingerprint (correction #13).
- Source suppressions are an immediately preceding standalone line comment:
  `// nsd-ignore[JSTS-EMPTY-CATCH]: non-empty reason`
  Only the six concrete built-in **rule IDs** are valid targets. Same-line,
  block-comment, unknown-rule and missing-reason directives are invalid and
  raise `NSD-S102`.
  **`E101`, `E102` and `V102` are deliberately not suppressible in source** —
  complexity and duplication are the anti-gaming core. Trusted policy may
  downgrade them to exit-neutral `warn` or disable them with `off`.
  `--allow-new-suppressions` affects `S101` alone (correction #16).
- Suppression identity includes the rule ID and matched underlying finding,
  not just directive text or location. Match underlying findings before applying
  suppressions. Moving a finding together with its unchanged directive remains
  tolerated; transferring that directive to an unmatched finding is a new
  suppression and raises `S101` (A9).

### Required clone coverage

The immutable source-file ceiling is 1 MiB; exactly 1 MiB is accepted. An
included changed file exceeding it raises `A102`. When `V102` is enabled
(`deny` or `warn`), the same fail-closed requirement covers included unchanged
files needed for repository-wide clone comparison: size, invalid encoding,
or missing analyzer capability must raise `A102`, never silently omit the
file from the index and report a pass (A10). A trusted exclusion removes the
file from scope. `V102: off` removes this additional unchanged-file obligation,
not analysis requirements for changed files. Existing tolerance for mapped
legacy parse damage remains governed by the parse-damage rules below.

---

## Parse recovery and the skipped-file taxonomy

The change that answers "too much is being skipped". The skip count is real
but almost entirely benign; the genuine defects are *reporting* and
*whole-file dropping*.

### What `js-ts-fixture-01`'s 30,864 skips actually are

| reason | count | assessment |
|---|---|---|
| `gitignore` | 30,318 | **29,893 under `.claude/worktrees/`** (nested worktrees of the same repo) + 425 `.venv`. Correctly excluded |
| `test` | 491 | policy, `scan` default |
| `generated_code` | 34 | policy |
| `parse_syntax_error` | **16** | **the only real analysis loss — 0.05 %** |
| `user_exclude` | 5 | invocation |

Two consequences:

1. **`skipped_files` conflates three unrelated things** — "policy says don't
   look", "vendored noise", and "we tried and failed". Only the third bears on
   trust in a score. `report.json` gains **separate counts per reason**, and
   `incomplete` is driven **only** by analysis failure, never by policy
   exclusion. `docs/measurements.md`'s existing caveat that the cell "is not
   itself the parse-failure count, only an upper bound" stops being necessary.
2. **Nested checkouts get a built-in exclusion.** A directory containing its
   own `.git` (worktree or clone) is excluded by `scan` regardless of
   `.gitignore`. Without this, one `.gitignore` edit exposes ~29,893
   near-identical files and detonates clone detection. `check` is already
   immune: its discovery reads Git trees/indexes, which never contain them.

### The 16 parse failures, and why salvage beats a parser swap

All 16 reproduce under the repo's own extension mapping
(`src/model.rs:124-127`). Damage is **27 lines of 1,409 — 1.92 %** — and
**44 of 61 callables lie entirely outside the damaged spans**:

```
ResumeDelete/index.jsx           131 lines | 1 damaged line (0.8%) | 7/9 callables clean
CartTableProductClickme/…tsx     194 lines | 1 damaged line (0.5%) | 9/10 callables clean
ResumeHeader/ResumePersonalGender.jsx 84 lines | 1 damaged line (1.2%) | 6/7 callables clean
…                                TOTAL 1409 lines, 27 damaged (1.92%), 44/61 recoverable
```

Root cause, reduced to one line (the same discriminator style the ledger uses
for the Java varargs bug):

| input | result |
|---|---|
| `const A = () => <L to="/x?a=1&b=2" />;` | **ERROR** |
| `const A = () => <L to="/x?a=1&amp;b=2" />;` | OK |
| `const A = () => <L to="/x?a=1&b;=2" />;` | OK |
| `const A = () => <L to={"/x?a=1&b=2"} />;` | OK |

A **bare `&` inside a double-quoted JSX attribute string, followed by
identifier characters with no terminating `;`** — i.e. an unterminated HTML
character reference. Per JSX semantics a bare `&` is legal attribute text, so
this is an upstream grammar bug. It is the `&utm_source=` / `&from=`
query-string idiom, which is why it clusters in marketing-link components.
`tree-sitter-typescript` is at its newest published release (0.23.2,
2024-11-11), so **no version bump fixes it**.

**Therefore v1 salvages instead of dropping.** Preserve tree-sitter error and
missing-node spans rather than discarding a file on `root.has_error()`.
Analyze only syntax entities proven not to intersect a damaged region; any
changed line lacking reliable coverage raises `NSD-A101`. This recovers ~72 %
of the callables in the affected files, is language-agnostic (it helps Java
identically), and needs no new dependency. Fail-closed is preserved: the
component enclosing a bad attribute stays unmeasured, which is correct.

Under D-IR salvage becomes an **IR-level** capability rather than a
tree-sitter-specific one, which is why damage spans are an IR invariant: the
lowering translates tree-sitter's error and missing nodes into typed IR
damage, and every analyzer honours it the same way in both languages.

Three known parse-damage classes are fixtured and tracked by name:

| class | corpus | count | v1 disposition |
|---|---|---|---|
| Java type-use annotation on a varargs parameter (`Class<?> @Nullable ... cs`) | spring-framework | 55 | **must disappear** under orchard — M0 gate |
| TS parameter literally named `using` | angular | 3 | remains, explained, rides base-failed rule |
| JSX attribute with unterminated `&` entity | js-ts-fixture-01 | 16 | remains, **now salvaged** rather than dropped |

---

## Architecture

### The D-IR pipeline

```
detect language  ->  tree-sitter parse  ->  lowering  ->  normalized IR  ->  analyzers
                     orchard | ts | js       src/lower/   spans, decision     cc / sloc
                                             java|jsts    kinds, damage       clones / rules
```

The point of the boundary is **not** parser portability for its own sake — it
is that today the grammar's node naming *is* the measurement contract. `cc`,
D11, `normalized_statement_tokens` and three of six rules read `node.kind()`
strings directly, which is why swapping one Java grammar forces a full
re-baseline, three hand-maintained D11 copies, and two hand-checked callables
to prove the delta was explained. Under D-IR a grammar swap is re-verification
of **one lowering** against a conformance corpus.

It also moves cross-language comparability somewhere testable. "Does a Java
`if` weigh what a TS `if` weighs?" stops being an emergent accident of two
node-kind tables and becomes a property of the lowering, asserted by a suite.
This is what makes `overall.erosion` defensible as one number.

**What the IR must carry**, driven by its four consumers:

| consumer | requirement on the IR |
|---|---|
| `cc` | a uniform decision-node kind enum (branch / loop / case / catch / ternary / `&&` / `\|\|`) with one weight table, not per-grammar strings |
| D11 / SLOC | every node carries a byte-and-line `Span` back to original source, plus an `executable` flag |
| clones | a statement-token stream faithful enough that two copy-pasted blocks fingerprint identically |
| the six rules | structural predicates (terminator-ness, block membership, catch bodies) expressible without grammar strings |
| salvage | typed damage spans, so entities provably clear of damage stay analyzable |

**Clone detection sets the IR's floor and is therefore built first.** An IR
minimal enough to be genuinely uniform across Java and TS risks being too
lossy to fingerprint clones; one faithful enough drifts back toward a full AST
with a shared vocabulary. That tension is the real design work, so M0b
prototypes clones against the IR before the IR's shape is frozen.

### Algorithms preserved, node access retargeted

Under D-IR these keep their **algorithms and their numbers** but stop reading
tree-sitter nodes. This is a retarget, not a rewrite: the measurement-neutrality
gate in M0b requires byte-identical output before and after.

| Path | Preserved | Retargeted |
|---|---|---|
| `src/metrics/mod.rs` | `mass`, `erosion`, `rank_top_callables`, the `decision_weight` *table* | `scan_file`, `scan_callable_body` walk IR, weights key off the IR decision enum |
| `src/clones/mod.rs` | `enumerate_candidates`, `drop_subsumed_groups`, `redundant_occurrences`, fingerprint width | `normalized_statement_tokens` emits from IR statements (**the constraint that sets the IR's floor**) |
| `src/rules/mod.rs` | six rules, `ALL_RULE_IDS`, `compute_verbosity` | predicates become IR structural queries; `file_language_lines` reads IR spans |

### Reused unchanged (port as-is)

| Path | Provides |
|---|---|
| `src/discover.rs` | walk, `.gitignore` handling, default exclusions, test globs |
| `src/report/html.rs` | standalone HTML report (`scan` only) |
| `docs/cc-rules.md`, `docs/wasteful-rules.md`, `docs/clone-detection.md` | **already the M0 measurement contract** — update to describe IR kinds instead of grammar node names, don't rewrite the semantics |

### Modified

- `src/parse/mod.rs:94-97` — Java grammar → orchard (`java_orchard`); error-span
  preservation replaces whole-file `has_error()` rejection.
- `src/clones/mod.rs:427-434` — `DefaultHasher` → versioned BLAKE3.
- `src/model.rs` — `Callable::end_line`; `SkipReason` split so analysis failure
  is distinguishable from policy exclusion.
- `src/rules/mod.rs` — verbosity numerator becomes the position union.
- `src/discover.rs` — nested-checkout exclusion.
- `src/cli.rs` — subcommand tree; `--include-tests`/`--exclude` stay `scan`
  flags (`check` scopes via `nsd.yml` `include`/`exclude`); the clone threshold
  is `measurement.min_clone_lines` for both, with `--min-clone-lines`
  overriding it in `scan` (A2).

### New

```
src/ir/        normalized node kinds, decision enum, Span, executable flags,
               damage spans, IR version constant
src/lower/     java.rs, jsts.rs — tree-sitter tree -> IR, per-lowering
               version constant
src/git/       snapshot.rs (Commit/Index/Worktree/Session), diff.rs, mergebase.rs
src/identity/  callable identity, tiers 1-3 matching, body fingerprints
src/policy/    the twelve NSD codes, config trust, exit-code resolution
src/cache/     blob-OID-keyed analysis cache, eviction
src/suppress/  preceding-line parser, new-suppression detection
src/format/    agent.rs, canonical JSON (deterministic ordering)
```

`src/lower/` holds exactly two lowerings, both over tree-sitter, written as
plain functions — **no `Frontend` trait, no plugin system, no second parser
family**. The seam earns its keep on tree-sitter alone (grammar swaps become
one-lowering changes; comparability becomes testable). That it would also make
a future parser additive is a side effect, not a requirement, and no v1 code
is shaped around it.

---

## Work plan

### M0 — Port, IR, grammar, freeze

The ordering is the point: **the IR lands before the grammar swap, against the
current grammars.** That isolates two variables which would otherwise move
together. If the IR is introduced and the grammar swapped in one step, any
delta is unattributable — exactly the failure mode M0 exists to prevent.

#### M0a — Port and consolidate

1. Use the initialized `~/pet/nsd` handoff repository (initialize it only if
   absent); preserve its root plans and agent instructions. Selectively import
   the engine from the A5 pin as specified in `nsd-plan-implementation.md`;
   rename crate/bin/docs/config/profile to `nsd` / `nsd-v1`, preserving
   attribution files. The root `nsd-plan-final.md` remains the single spec;
   documentation should link to it instead of duplicating it.
2. **Consolidate the three D11 executable-line implementations into one shared
   function** (`src/metrics/mod.rs`, `src/clones/mod.rs`,
   `src/rules/mod.rs:335-380`/`:420-425`). Still a hard prerequisite, and now
   also the thing that becomes the IR's single `executable` rule.
3. Replace `DefaultHasher` with versioned BLAKE3 before anything persists a
   fingerprint.

#### M0b — IR, on the existing grammars

4. **Prototype clone lowering first.** Before freezing IR shape, emit
   `normalized_statement_tokens` from a draft IR and require it to reproduce
   today's clone groups on `java-fixture-01@c6671504…` **exactly**. Clones set the
   IR's floor (see *Architecture*); if the IR cannot carry them, nothing
   downstream is worth building.
5. Define `src/ir/`; write the Java and JS/TS lowerings in `src/lower/` as
   plain functions over tree-sitter trees. Both must emit typed damage spans.
6. Retarget `cc`, D11/SLOC, clones and the six rules onto the IR.
7. Implement error-span salvage and the `SkipReason` split — both now IR-level
   and therefore language-agnostic by construction.
8. **Measurement-neutrality gate.** With grammars unchanged (`tree-sitter-java`
   0.23.5, `tree-sitter` 0.25.10), the IR build must produce **byte-identical**
   `report.json` on unaffected fixtures from the ten suites and against
   `java-fixture-01@c6671504…` (overall `0.13313797553867948`, java
   `0.13294913151043064`, `incomplete: false`). The java-fixture-01 comparison
   runs locally against the archived full `report.json` in `agent_slope`,
   from the checkout path that file records; the committed digest (A1) is
   what CI and later readers compare against. For suite fixtures, run the
   pre-IR and IR binaries against the **same fixed fixture checkout**, with
   identical target arguments, revision SHA, dirty state, and scan settings;
   do not compare reports from each crate's own `CARGO_MANIFEST_DIR` fixtures.
   Retain legacy JSON serialization through this gate; add new report fields
   and widened `end_line` spans only afterward. Any measurement delta outside
   the declared exception below is an IR fidelity bug and blocks M0c; do not
   baseline it away.
   *Exception, declared up front:* salvage and the `SkipReason` split
   intentionally change existing malformed fixtures, including
   `js-ts-fixture-01` and `js-ts-fixture-02`. Assert only those declared deltas
   explicitly; unaffected measurements must remain identical. Run the strict gate on
   `java-fixture-01`, whose scan has no parse failures, so neither feature can
   mask an IR defect.

#### M0c — Grammar swap, now a one-lowering change

9. Swap to `tree-sitter-java-orchard` 0.5.18; bump `tree-sitter` to 0.27.0;
   declare MSRV 1.90 (D-GRAMMAR — elective, taken knowingly).
10. **Quantify the re-baseline.** Re-run `scripts/perf_scan.sh` on the pinned
    Spring+Angular fixture; record in `docs/measurements.md` the parse-failure
    movement (55 Java → 0 required; 3 TS `using` remain) and the Java
    SLOC/mass/erosion delta attributable to orchard's named
    `visibility`/`modifier` nodes. Because M0b froze the analyzers against the
    IR, this delta is now **wholly attributable to the Java lowering** — review
    the lowering diff, not four analyzers. The delta must still be
    **explained**, not merely observed, on at least two hand-checked callables
    with exact before/after SLOC/CC/mass. Record per-rule and clone counts, not
    only aggregate ratios. **Stop M0 without freezing if the 55 do not clear.**
11. Re-baseline fixture expectations; update the three `docs/*.md` contracts to
    describe IR kinds rather than grammar node names.
12. Freeze `nsd-v1`, fingerprint including IR version, both lowering versions,
    the three grammar versions, the `tree-sitter` runtime version, the rule
    catalog and the effective clone configuration (correction #18, A2).

Clear the cheap ledger rows while these files are open: `erosion` returning
`-0.0`, `BTreeSet<usize>` → sorted `Vec`, the `rposition` and `always_returns`
mutation survivors, the `LanguageFamily::JsTs` guard at `src/rules/mod.rs:339`,
and `tests/report_html.rs`'s `#L1-L1` anchor assertion (which goes vacuous once
`end_line` widens spans — fix with the `end_line` row, not after).

### M1–M2 — Snapshots, diffs, identity

- `git2`-backed commit, index and worktree snapshots over repository-relative
  UTF-8 bytes. `--staged` reads the index, never the dirty worktree.
- Git-backed discovery comes from trees/indexes, independent of candidate
  `.gitignore`. Worktree mode overlays tracked changes and eligible untracked
  files using base policy.
- Merge-base resolution with an actionable error on a shallow clone
  (`NSD-G101`).
- Centralize line mapping as one shared primitive for parse errors, findings,
  suppressions, callables and clone attribution.
- Handle additions, deletions, modifications, renames, symlinks, submodules,
  invalid UTF-8 and large files deterministically.
- Replace `<anonymous>@<line>` with a line-independent identity; the current
  form makes every insertion above an anonymous callable look like delete+add.
- Report mapped parser gaps and coverage explicitly. Ratios use *analyzed*
  executable lines and carry completeness metadata.

### M3–M5 — Policy, suppressions, clones, cache

- E101/E102 after callable matching; deletions and improvements never fail.
- V101 after diff-aware finding matching, so line-only movement is not a
  regression.
- Detect new suppressions through diff mapping and underlying finding matches.
  Moving a finding with its unchanged directive is not new; transferring the
  directive to an unmatched finding raises `S101` (A9). Invalid/unused directives
  raise `S102`.
- Enforce required clone-index coverage for unchanged included files while
  `V102` is enabled: size, encoding, and capability failures raise `A102` (A10).
  Preserve the separate mapped legacy parse-damage tolerance.
- Compare candidate changed clone occurrences against unchanged base content
  and other candidate changes; exclude the replaced base version of a modified
  path. Apply maximal-group reduction before V102 emission so contiguous
  sub-runs do not flood diagnostics.
- Persist path-independent per-blob analysis under the repository's common Git
  directory, keyed `blob OID + grammar/language + measurement fingerprint`
  (so linked worktrees share it). Payload: parse coverage, callable metrics,
  rule findings, executable lines, clone candidates.
- Worktree files hash by Git blob-hash semantics. Writes atomic; corruption or
  cache-version mismatch is a miss; cache I/O failure warns and recomputes
  rather than affecting correctness.
- Best-effort LRU eviction, 1 GiB / 30-day default. Eviction is real work:
  `enumerate_candidates` emits every contiguous sub-run before
  `min_clone_lines` prunes, so the payload is large. Exact staged repository
  ratios assemble from cached unchanged blobs plus newly analyzed candidate
  blobs.

### M6–M7 — Output, CI hardening

- Canonical report `schema_version: 1`, distinguishing `scan` and `check`
  scopes.
- Canonical JSON: complete entity/diagnostic set, stable ordering, fixed
  numeric serialization, repository-relative paths, snapshot IDs, configuration
  and measurement fingerprints, **per-reason skip counts**, and no timestamps,
  absolute checkout paths or raw source excerpts.
- Terminal and agent formats default to 50 and 30 diagnostics, with
  deterministic omitted counts. Exit status always reflects the complete result.
- Scan HTML stays standalone; each excerpt capped at 20 lines / 4 KiB with an
  explicit truncation marker.
- Built-in exclusions cover dependency/build/generated/minified/WebJar paths
  **and nested checkouts**. `scan` also excludes test, fixture, `e2e` and QA
  paths unless requested; `check` has no default test exclusion, though trusted
  base config may explicitly exclude paths.
- `--allow-new-suppressions` plumbing, full-scan path, DoD sweep.

---

## Spec deltas

Sections of `LSD_plan_openai_v2.md` that do **not** survive:

- **§8, §29 (jscpd v5 engine)** — deleted. Premise was that scb-check is the
  semantic reference; it is not. scb-check uses AST-subtree hashing
  (`src/scb_check/analysis/clones.py`), which `src/clones/mod.rs` already
  implements.
- **§5, §37 `reference` level, §68, §73** — cut. Reference compatibility is
  permanently unreachable: scb-check has no Java support, and its 197 ast-grep
  + 2 structural rules are Python-only.
- **§24 Python fixtures, §37 Python rows** — cut with Python.
- **§58 watch mode** — deferred to v1.1.
- **§89 project identity** — `lsd` → `nsd` throughout.
- **§84 M12** (`LSD_plan_openai_v2.md:2880`) — reduced to M5. Erosion and rules
  are *local* detectors and diff at `O(changed files)`; only clone detection is
  inherently global, so only it needs a cache.

---

## Rejected alternatives

Kept with their evidence so none is re-proposed.

- **oxc / oxlint as a *metrics source* — rejected permanently.** Its
  `complexity` rule counts optional chaining, default parameters and
  destructuring defaults — which Java cannot have — so a shared `mass` scale
  would not measure one thing. The value lives in a private struct, escaping
  only inside a diagnostic string; oxc has no SLOC and no token nodes. D-IR
  does not revive this: nsd computes its own metrics over its own IR, and
  never imports another tool's complexity number.
- **oxc as a *parser* — not in v1, and not on the v1 roadmap.** Recorded only
  so the research is not redone: `oxc_parser` is at **0.151.0** (2026-09-21) on
  a weekly 0.x cadence with routine breaking changes, against a tool whose
  value is a frozen reproducible measurement, and it carries MSRV 1.96 versus
  the 1.90 v1 commits to. The 16 js-ts-fixture-01 files it would fix are 0.05 % of skips
  and 1.92 % damaged by line, and salvage already recovers 72 % of their
  callables with no new dependency. **Trigger to reopen, and the only one:**
  parse loss exceeding ~5 % of analyzed JS/TS lines. Until then tree-sitter is
  the parser for every language and this bullet is closed.
- **Python frontends in v1 — deferred with Python itself.** Recorded so the
  options are not re-researched: `ruff_python_parser` is **0.0.14**, three
  releases in the fortnight to 2026-09-16, self-described *"an internal
  component crate of Ruff"* — no semver, no public-API promise, MSRV 1.96.
  `rustpython-parser` is 0.4.0, last released **2024-08-06**; ruff forked from
  it and moved on, so it is the `rust-code-analysis` failure mode again.
  `tree-sitter-python` 0.25.0 (2025-09-11) is maintained and needs no new
  frontend family — it is the default choice **if and when** Python returns,
  which is a scope decision (no validated reference for Python numbers), not a
  parser one.
- **`NSD-E102` as a relative mass threshold.** No threshold works. CC 12 with
  SLOC 9→10 (should pass) moves mass **+5.41 %**; CC 25→26 at SLOC 200 (should
  block) moves it **+4.00 %**. The orderings cross, so any threshold admitting
  the first admits every new branch at CC ≥ 19. The adopted absolute-SLOC form
  reproduces the intended boundaries: base 9 → 10 passes (`1 > 1` false),
  base 9 → 11 fails (`2 > 1`).
- **ast-grep as the parser layer.** It *is* tree-sitter — re-exports its types
  and pins `tree-sitter-java ^0.23.0`, resolving to the same buggy 0.23.5. No
  complexity, no SLOC, no clone detection.
- **`mozilla/rust-code-analysis`.** Genuinely close fit (per-`FuncSpace` CC +
  SLOC for Java/JS/TS/TSX on tree-sitter), but crates.io is frozen at 0.0.25
  (Jan 2023) so it needs a git pin to an unmaintained tree; MPL-2.0 file-level
  copyleft; no clone detection or rule engine; it pins the same
  `tree-sitter-java` 0.23.5; and its Java-vs-JS CC comparability is unverified.
- **Migrating to Biome.** The only alternative preserving the architecture, but
  its JS crates are stale on crates.io (0.5.7, Mar 2024), requiring pinned git
  revs across six crates with no semver. Keep in reserve.
- **Preprocessing JSX sources to escape bare `&`.** Rejected: rewriting bytes
  before parsing shifts every span, breaking line mapping, suppression
  attribution and clone offsets — the precise machinery M1–M2 centralizes.
- **Forking `tree-sitter-typescript` to patch the JSX entity rule.** Held in
  reserve behind salvage. It means vendoring `grammar.js` plus
  `tree-sitter generate` in the build, and owning a grammar fork forever, to
  recover 16 files that salvage already largely recovers. Reconsider only
  together with the `using` gap, and only if upstream stays dead.

---

## Test and acceptance plan

**Per milestone.** All **ten** existing suites (`tests/{cli,clones,discover,
metrics,rules,verbosity,report_json,report_html,e2e_local,e2e_github}.rs`) stay
green; M0 re-baselines Java expectations and must state each changed number and
why.

**IR conformance (new, and the gate M0b turns on)**

- **Measurement neutrality:** with grammars unchanged, the IR build produces
  byte-identical legacy `report.json` to the pre-IR build on unaffected fixtures
  from all ten suites and `java-fixture-01@c6671504…`. Both binaries scan the
  same fixed checkout with identical target/revision/settings metadata.
  Existing malformed fixtures permit only explicitly asserted salvage and
  skip-taxonomy deltas, with unaffected measurements unchanged. Blocks M0c.
- **Clone floor:** IR-emitted `normalized_statement_tokens` reproduce the
  pre-IR clone groups on `java-fixture-01` exactly, including group membership
  and `drop_subsumed_groups` outcomes.
- **Cross-language weight parity:** a corpus of matched constructs — `if`,
  `for`, `while`, `switch`/`case`, `catch`, ternary, `&&`, `||` — written once
  in Java and once in TS, asserting identical CC contribution and identical
  executable-line counts per construct. This is the suite that makes
  `overall.erosion` defensible as a single number.
- **Lowering isolation:** no symbol under `src/ir/` or any analyzer names a
  grammar node kind (enforced by a source-level test). The reason is v1's own
  — a leaked node-kind string silently re-couples a metric to the grammar and
  quietly restores the re-baseline cost D-IR exists to remove.
- **Damage-span contract:** lowering returns typed damage spans on malformed
  input for each of the three known damage classes.
- **Fingerprint coverage:** changing the IR version, either lowering version,
  or any grammar version changes the measurement fingerprint; changing none of
  them does not.

**New focused suites**

- every snapshot mode; shallow/missing merge base; staged rename/delete/add
  (rename detection asserted via the configured `rename_threshold = 50` plus
  one clear rename and one clear non-rename fixture — A6); untracked worktree
  files; dirty-worktree isolation;
- callable overloads, nesting, anonymous callbacks, moves, line-only changes,
  exact-body ambiguity, E101 threshold crossings, and every E102 boundary
  including **`9→10` pass** and **`9→11` fail**;
- parse-span mapping; changed lines near legacy errors; worse coverage; total
  parse loss; invalid UTF-8; capability failure;
- **parse salvage:** a fixture per damage class — Java varargs annotation, TS
  `using`, **JSX unterminated `&`** — asserting that callables outside the
  damaged span are measured and those intersecting it are not;
- **skip taxonomy:** per-reason counts; `incomplete` driven only by analysis
  failure; a nested `.git` checkout excluded even when not gitignored;
- moved/new/invalid/unused suppressions, `S102`, the invocation-only escape,
  and that `E101`/`E102`/`V102` reject in-source suppression;
- moving a finding with its unchanged directive passes; transferring an
  existing directive to an unmatched finding raises `S101`;
- unchanged included clone inputs exceeding 1 MiB, invalid encoding, or
  unavailable capability raise `A102` under both `V102: deny` and `warn`;
  trusted exclusion removes the obligation, `off` removes only the additional
  unchanged-file obligation, and mapped legacy parse damage remains tolerated;
- candidate config weakening, first-config defaults, trusted override, invalid
  config, a trusted `--config` inside the checkout, reached through an ancestor
  symlink or a filesystem alias, refused with `C102` (A11), candidate
  `.gitignore` changes, and **`C101` not altering exit 0**;
- clones against untouched files, changed-file replacement, new occurrence,
  material extension (base `10`: `+1` passes, `+2` fails; base `100`: `+10`
  passes, `+11` fails), **cross-file move passes and cross-file copy fails**
  (A3), overlap reduction, verbosity union semantics;
- cold/warm cache equivalence, wrong-language same-blob isolation, corruption
  recovery, concurrent writers, eviction, and that no persisted key derives
  from `DefaultHasher`;
- byte-identical JSON across repeated runs, checkout roots, thread counts and
  cache states.

**Fixtures and end-to-end**

- Re-run the pinned Spring+Angular performance fixture; record cold/warm time,
  peak RSS, cache size, parse coverage and exact rebaseline deltas.
- **`java-fixture-01@c6671504394b7c862dac032bc7c7364ad3af8b7f`** is the Java E2E
  golden — the only recorded clean, complete scan
  (`dirty: false`, `incomplete: false`, overall `0.13313797553867948`).
- **`js-ts-fixture-01@1c61294688f50bc8770dc251b96d528e905fe0ce`** is the JS/TS E2E fixture.
  Its `0.26197032832368317` is **not** a golden while `incomplete: true`; the
  acceptance criterion is that salvage moves parse failures 16 → 0 *dropped
  files* and that `incomplete` reflects only residual damaged spans.
- **`js-ts-fixture-02`** is re-scanned only from a recorded **clean** commit; `0.5399` is
  never a baseline (dirty at `257b564`).
- Scratch-repository acceptance must prove E101, E102, V101, V102, S101, A101,
  exits `0/1/2/3`, base-config trust, unchanged staged verdict under unstaged
  edits, worktree mode, and a self-check of the `nsd` repository in CI.

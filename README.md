# NoSlopeDude (`nsd`)

**Stop new code slop at the change that introduces it.** `nsd` scans a codebase for structural waste, then compares a proposed change with its Git base. Existing debt shows up in reports; new high complexity, duplicated blocks, and wasteful code can fail a pre-commit hook or CI job.

```sh
nsd check --staged                 # review exactly what you staged
nsd check --base origin/main       # gate a branch against its merge base
nsd scan . --output nsd-report     # explore the codebase in HTML and JSON
```

The [SlopCodeBench paper](https://arxiv.org/abs/2603.24755) measures how code quality degrades as coding agents repeatedly extend a project. It motivates two of `nsd`'s signals: **erosion**, the share of complexity mass in callables above cyclomatic complexity 10, and **verbosity**, the share of executable lines occupied by clones or rule findings. `nsd` turns those snapshot signals into a change gate: it asks *did this change make the structure worse?* A legacy codebase can adopt the gate without first cleaning up every old finding.

## How it helps

Imagine a repository that already has 40 complex functions. A snapshot score tells you there is debt, but failing every commit on that score would stop unrelated work. Run `nsd scan . --output nsd-report` to see where the debt is. Then use `nsd check --staged` while coding and `nsd check --base origin/main` in CI. A change that leaves those 40 functions alone passes; a newly complex function or a new duplicated block gets a source-level diagnostic before it adds to the cleanup queue.

That gives teams a practical path to improvement:

1. **See the current shape.** The HTML report shows complex callables, clone groups, rule findings, and files that could not be analyzed.
2. **Catch regressions where they start.** The Git comparison attributes a new finding to the staged change or branch instead of blaming the author for old code.
3. **Keep the gate honest.** The base revision's policy judges the change, so a PR cannot disable its own checks by editing `nsd.yml`. Analysis errors also fail the check rather than silently passing.

This is useful when coding agents produce many changes quickly: reviewers can focus on the specific new structural problems, while the existing backlog remains visible in the scan report.

## Get started

Build from this repository:

```sh
cargo build --release --locked
./target/release/nsd scan . --output nsd-report
./target/release/nsd check --staged
```

The scan writes `nsd-report/report.html` for browsing and `report.json` for tooling. `scan` also accepts a public GitHub URL:

```sh
nsd scan https://github.com/owner/repo --output nsd-report --include-tests
```

A [prebuilt static Linux binary](https://github.com/tp4k/NoSlopeDude/releases) is available in tagged releases; [download, checksum, and provenance instructions](docs/cli.md#release-binary) cover CI installation. Run `nsd --help`, `nsd scan --help`, or `nsd check --help` for the installed binary's syntax.

## What it catches

`nsd` parses Java, JavaScript, TypeScript, JSX, TSX, and Rust with tree-sitter, lowers their syntax trees into a common intermediate representation, and measures callables and duplicate code from that representation. Java and JS/TS also get six conservative wasteful-code rules: unreachable statements after a terminator, empty catch blocks, and redundant `else` after return or throw. Rust gets complexity and clone analysis, but no wasteful-code rules yet.

For each callable, `mass = cyclomatic complexity × √(executable SLOC)`. Erosion is the fraction of total mass in callables with complexity above 10. Verbosity counts distinct executable lines covered by redundant clone occurrences or rule findings, divided by scanned executable lines. These ratios help explain a scan; **the check gates specific new regressions, not a whole-repository ratio**.

For example, if an existing method has complexity 14, `nsd check` does not fail merely because it exists. A newly added method crossing complexity 10 raises `NSD-E101`; making that already complex method worse can raise `NSD-E102`. New rule findings raise `NSD-V101`, and new qualifying duplicate blocks raise `NSD-V102`. Diagnostics point to the affected source; the [CLI reference](docs/cli.md#check) explains their output.

## Choose the right command

| Goal | Command | Candidate compared with |
|---|---|---|
| Explore a local checkout or public GitHub repository | `nsd scan <target> --output <dir>` | No comparison; writes HTML and JSON |
| Gate a commit before creating it | `nsd check --staged` | Git index versus `HEAD` |
| Gate a committed branch in CI | `nsd check --base origin/main` | `HEAD` versus the merge base with `origin/main` |
| Inspect local edits, including eligible untracked files | `nsd check --base origin/main --worktree` | Worktree versus that merge base |

`--staged` reads the **index**, so unstaged edits cannot change its verdict. `--worktree` requires `--base`. Run `check` from the repository root. CI needs enough fetched history to find the merge base.

### Useful flags

| Flag | Command | Effect |
|---|---|---|
| `--include-tests` | `scan` | Include test files in the snapshot report; `scan` excludes them by default. |
| `--exclude <glob>` | `scan` | Exclude a path pattern; repeat the flag for more patterns. |
| `--min-clone-lines <n>` | `scan` | Override the clone line threshold for this scan (default 10, or root `nsd.yml`). |
| `--format terminal\|json\|agent\|github` | `check` | Choose readable diagnostics, uncapped canonical JSON, violations only, or GitHub Actions annotations. |
| `--json-out <path>` | `check` | Also save the canonical JSON report while keeping the selected stdout format. |
| `--config <path>` | `check` | Replace repository policy with a trusted config file **outside** the candidate checkout. |
| `--allow-new-suppressions` | `check` | Let new suppression directives through for this invocation only; other denied findings still fail. |

Examples:

```sh
nsd scan . --output nsd-report --exclude 'vendor/**' --min-clone-lines 15
nsd check --staged --format agent
nsd check --base origin/main --format github --json-out nsd.json
```

`check` exits `0` on pass, `1` for a denied regression, `2` for an analysis/configuration/snapshot error, and `3` when both kinds occur. Warnings and informational diagnostics do not fail the gate. The [CLI reference](docs/cli.md) has the full flag behavior, output formats, defaults, and ready-to-copy hook and CI commands.

## Policy that a change cannot weaken

Put `nsd.yml` at the repository root to set scope, the clone threshold, policy severities, and output limits. For example:

```yaml
version: 1
exclude:
  - vendor/**
measurement:
  min_clone_lines: 10
policy:
  NSD-E101: deny
  NSD-E102: deny
  NSD-V101: deny
  NSD-V102: deny
  NSD-S102: warn
```

For `check`, the **base snapshot's** policy judges the candidate. A change to `nsd.yml` is reported, but cannot turn off its own gate; its settings apply to later changes. An external `--config` is an explicit trusted override. `scan` reads only `measurement.min_clone_lines` from its target root's `nsd.yml`; its `--min-clone-lines` flag overrides that value.

## Research

`nsd` draws its erosion and verbosity ideas from [*SlopCodeBench: Benchmarking How Coding Agents Degrade Over Long-Horizon Iterative Tasks*](https://arxiv.org/abs/2603.24755). Its change gate is a separate application of those measurements.

# CLI

## `scan`

```
nsd scan <folder-or-github-url> --output <directory> [--include-tests] [--exclude <glob>]... [--min-clone-lines <n>]
```

Scans a local folder or a public GitHub repository (a shallow, single-branch
clone of the default branch) and reports on `.java`, `.js`, `.jsx`, `.mjs`,
`.cjs`, `.ts`, and `.tsx` source files.

### Arguments

- `<folder-or-github-url>` — a local filesystem path, or a `https://github.com/<owner>/<repo>` URL. Any other host, a private repository, or an SSH remote is not supported.

### Flags

- `--output <directory>` (**required**) — where `report.json` and `report.html` are written. Created if it does not already exist.
- `--include-tests` — include test files in the scan. Off by default.
- `--exclude <glob>` — an additional glob to exclude from the scan; may be repeated.
- `--min-clone-lines <n>` — the minimum number of duplicated source lines to report as a clone. Default: `10`.

### Default exclusions

Applied regardless of `.gitignore`, unless overridden by scanning a path that itself matches none of them:

- **Dependencies and build output**: `node_modules/`, `target/`, `build/`, `dist/`, `out/`, `bin/`, `.gradle/`, `.mvn/`, `vendor/`, `coverage/`, `.next/`, `.nuxt/`, `**/META-INF/resources/webjars/**` (WebJar resources, kept even with `--include-tests`)
- **Generated code**: `**/generated/**`, `**/gen/**`, `*.min.js`, `*.bundle.js`, `*_pb.js`, `*.d.ts`
- **Tests** (unless `--include-tests`): `**/test/**`, `**/tests/**`, `**/__tests__/**`, `**/__mocks__/**`, `**/testFixtures/**`, `**/src/test/**`, `*Test.java`, `*Tests.java`, `*TestCase.java`, `*.test.*`, `*.spec.*`, `**/fixture/**`, `**/fixtures/**`, `**/__fixtures__/**`, `**/e2e/**`, `**/qa/**`, `*.e2e-spec.*`
- **Nested checkouts**: a directory below the scan root that directly contains its own `.git` is not descended and is listed as one `nested_checkout` skip, unless a `.gitignore`, a default, test or user exclusion already covers the paths below it (those files keep their own reason). `--include-tests` therefore turns a `fixtures/` checkout from `test` rows into one `nested_checkout` row.

A scanned folder's own `.gitignore` is always respected, even when the folder is not itself a git repository.

### Exit codes

- `0` — the scan completed. A per-file parse failure degrades the affected scores (marked `incomplete: true`) rather than failing the scan.
- Nonzero — a fatal condition: an unreadable target, a failed clone, or an unwritable output directory.

## `check`

```
nsd check --staged      [--config <path>] [--format terminal|json|agent] [--allow-new-suppressions]
nsd check --base <ref> [--worktree] [--config <path>] [--format terminal|json|agent] [--allow-new-suppressions]
```

Compares a candidate change with its base under the base's own policy and prints one line per diagnostic, capped (see *Output caps*), to stdout. Run it at the root of the repository.

- `--staged` checks the Git index against `HEAD`; it never reads worktree source.
- `--base <ref>` checks `HEAD` against `merge-base(HEAD, <ref>)`; with `--worktree` the candidate is the working tree instead of `HEAD`.
- `--staged` and `--base` are mutually exclusive, one is required, and `--worktree` requires `--base`. A usage error exits `2` and prints nothing to stdout.
- `--format <terminal|json|agent>` picks the output, `terminal` by default. `json` prints one canonical JSON document instead of the lines below (see `docs/report-format.md` *Check JSON*) and is never capped; the exit code is the same for all three. `agent` prints the lines below for violations only (see *Output caps*).
- `--allow-new-suppressions` makes `NSD-S101` exit-neutral for this invocation only: the diagnostic still prints in `terminal` and appears in `json`, and `agent` leaves it out because it is no longer a violation. It affects `NSD-S101` alone, so `E101`, `E102`, `V101`, `V102` and a denied `S102` keep their exit contribution. `nsd.yml` has no key for it (any unknown key is `NSD-C102`), so only the command line can turn it on.
- `--config <path>` replaces the repository policy with a trusted file. A path that resolves inside the checkout is refused with `NSD-C102` and exit `2`, and is never read. Inside means the absolute path as given, or its directory resolved, or its symlinks resolved, or the way the OS opens it (so `..` after a symlink counts), lies under the checkout root (`.git/` included), or an existing directory above any of those is the checkout root under another name (same device and inode). That covers an alias of the root or of one of its ancestors, such as `/System/Volumes/Data/…` on macOS, a bind mount of the root, or a case variant on a case-insensitive volume. A missing file is judged the same way. Two cases are not refused: a hard link outside the checkout to a file inside it, and a bind mount of a checkout subdirectory at a path outside it. Use a file outside the checkout. The same-device-and-inode test runs on unix only; elsewhere only the path forms are compared.

Each line starts with the code. What follows depends on the code (`<span>` is `<repo-relative path>:<start>-<end>`):

| Codes | Rest of the line |
|---|---|
| `NSD-E101`, `NSD-E102`, `NSD-G102` | `<span> <callable>`, plus ` (base <span> cc <n> sloc <n>)` when the callable has a base match |
| `NSD-V101` | `<span> <rule id>` |
| `NSD-S101`, `NSD-S102` | `<repo-relative path>:<directive line>`, plus ` <rule id>` when the directive names one |
| `NSD-A101` | `<span>` |
| `NSD-A102` | `<repo-relative path> <reason>` |
| `NSD-V102` | `<span> (+<n> lines) matches <span>` |
| `NSD-C101` | `nsd.yml` |
| `NSD-C102`, `NSD-G101` | a free-text message, with no path in a fixed position |

Non-UTF-8 path bytes print as `%XX`, and control characters in any text print as `\u{..}`.

### Output caps

The caps come from the trusted configuration's `output` section: the base snapshot's `nsd.yml`, or `--config`. A candidate `nsd.yml` never changes its own caps. With no configuration resolved (including a refused `--config` and `NSD-G101`) the defaults apply: 50 for `terminal` (`max_terminal_diagnostics`) and 30 for `agent` (`max_agent_diagnostics`).

- `terminal` lists the first N diagnostics in the order of the complete result. When any were dropped it ends with one extra line, `omitted diagnostics: <count>`, the count in decimal. With nothing omitted there is no extra line.
- `agent` lists only violations: diagnostics with a non-zero exit contribution under the trusted policy (`warn`, `off` and informational ones such as `NSD-C101` are left out). It keeps the first N of them and, when any were dropped, ends with `omitted violations: <count>`, counting violations only.
- A cap of `0` lists no diagnostic line, only the omitted line when there is anything to omit.
- Neither extra line starts with `NSD-`. The same result always renders the same bytes.
- The exit code is always computed from the complete, uncapped result, so an omitted diagnostic still sets it.

### Exit codes

- `0` — no denied diagnostic. `warn` and informational diagnostics print but never change the exit.
- `1` — a denied regression.
- `2` — an analysis, configuration or snapshot error, including running outside a Git repository (`NSD-G101`).
- `3` — both.

## Pre-commit and CI

`nsd` ships no hook installer and no provider-specific workflow; copy the commands below into your own hook or pipeline. Each runs at the repository root.

### Pre-commit

Check what is staged. A hook body is one line; a non-zero exit blocks the commit.

```sh
nsd check --staged
```

For an agent or an editor integration, list violations only:

```sh
nsd check --staged --format agent
```

To let a reviewed new suppression directive through while every other denied code still blocks:

```sh
nsd check --staged --allow-new-suppressions
```

### CI

Check the branch head against its merge base with the target branch. CI must fetch enough history to contain that merge base (for example a full clone, not a shallow one), or the run fails with `NSD-G101` and exit `2`.

```sh
nsd check --base origin/main
```

For a machine-readable result, whose `summaries` carry the repository ratios (scope `full` while `NSD-V102` is not `off`, `changed` when it is `off`). The ratios are reported and never change the exit code:

```sh
nsd check --base origin/main --format json
```

Exit `0` passes the job, `1` and `3` fail it for a denied regression, and `2` and `3` fail it for an analysis or configuration error (see *Exit codes*).

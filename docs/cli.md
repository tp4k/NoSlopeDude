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

- **Dependencies and build output**: `node_modules/`, `target/`, `build/`, `dist/`, `out/`, `bin/`, `.gradle/`, `.mvn/`, `vendor/`, `coverage/`, `.next/`, `.nuxt/`
- **Generated code**: `**/generated/**`, `**/gen/**`, `*.min.js`, `*.bundle.js`, `*_pb.js`, `*.d.ts`
- **Tests** (unless `--include-tests`): `**/test/**`, `**/tests/**`, `**/__tests__/**`, `**/__mocks__/**`, `**/testFixtures/**`, `**/src/test/**`, `*Test.java`, `*Tests.java`, `*TestCase.java`, `*.test.*`, `*.spec.*`

A scanned folder's own `.gitignore` is always respected, even when the folder is not itself a git repository.

### Exit codes

- `0` — the scan completed. A per-file parse failure degrades the affected scores (marked `incomplete: true`) rather than failing the scan.
- Nonzero — a fatal condition: an unreadable target, a failed clone, or an unwritable output directory.

## `check`

```
nsd check --staged      [--config <path>]
nsd check --base <ref> [--worktree] [--config <path>]
```

Compares a candidate change with its base under the base's own policy and prints one line per diagnostic, uncapped, to stdout. Run it at the root of the repository.

- `--staged` checks the Git index against `HEAD`; it never reads worktree source.
- `--base <ref>` checks `HEAD` against `merge-base(HEAD, <ref>)`; with `--worktree` the candidate is the working tree instead of `HEAD`.
- `--staged` and `--base` are mutually exclusive, one is required, and `--worktree` requires `--base`. A usage error exits `2` and prints nothing to stdout.
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

### Exit codes

- `0` — no denied diagnostic. `warn` and informational diagnostics print but never change the exit.
- `1` — a denied regression.
- `2` — an analysis, configuration or snapshot error, including running outside a Git repository (`NSD-G101`).
- `3` — both.

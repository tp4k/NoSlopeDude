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

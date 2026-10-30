# Working on nsd

Read [README.md](README.md) for the product and command overview. Use the
documents in `docs/` for the current CLI, report, measurement, and analyzer
contracts. The code and tests determine implemented behavior when an older
document disagrees with them.

## Boundaries

- Keep one Rust crate with tree-sitter parsing, language-specific lowerings,
  normalized IR, and IR-based analyzers. Java, JS/TS, and Rust are supported.
- `--staged` reads the Git index, never unstaged worktree source. Checks use
  trusted base policy; candidate configuration cannot weaken its own gate.
- Match underlying findings before applying suppressions. Moving the original
  finding with its unchanged directive is tolerated; transferring a directive
  to an unmatched finding raises S101.
- With V102 set to deny or warn, included unchanged files required for clone
  comparison must satisfy size, encoding, and capability requirements;
  failures raise A102. Keep the separate legacy parse-damage tolerance.
- Source comments cannot suppress E101, E102, or V102. Trusted policy may
  downgrade them to warn or disable them with off.
- Persist only versioned stable fingerprints. Cache failure must recompute
  without changing verdicts. Staged cache entries derive from the analyzed
  snapshot bytes.

## Verification and fixtures

- Run checks relevant to the change and report their actual outcomes. Do not
  claim a private-fixture or MSRV check passed when it was not run.
- Keep private fixture names, checkout paths, source excerpts, and name
  mappings outside this repository. Authorship is unknown unless verified;
  never infer it from scores. Commit only sanitized numbers and hashes.
- `~/pet/agent_slope` is a read-only archive. Do not modify it or replace
  this repository's Git history. Do not import private reports, source, build
  outputs, or unrelated untracked files. Preserve existing user changes.

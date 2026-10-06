# Working on nsd

## Read first

Read [nsd-plan-final.md](nsd-plan-final.md) for the authoritative v1 contract,
then [nsd-plan-implementation.md](nsd-plan-implementation.md) for execution
order, configuration details, and acceptance checks. The final plan wins
on conflicts. Both include the approved final-review decisions; do not reopen
settled choices without new evidence or a user request.

Then read [docs/implementation-status.md](docs/implementation-status.md) — the
living record of which plan steps are already done, on which branch, and on
what evidence. It tells you where to start and what is blocked; the plans tell
you what the step means. Never infer "not started" from the plans alone.

## Recording progress

Every agent updates its own rows in
[docs/implementation-status.md](docs/implementation-status.md) as part of the
round that does the work — not in a later pass, not "at the end of the
milestone". The rules are in that file's *How to update this file* section;
the ones that are non-negotiable:

- Mark `[x]` only with a named check in this repository that passes, and cite
  the commit SHA plus the test, script, or document that proves it. Code that
  exists without a passing check is `[~]`.
- Never mark a row `[x]` on a red or unrun check, and never on a missing
  fixture or archive — that is `[!]`, with what is missing spelled out.
- Touch only the rows your workstream owns. Disagreement with someone else's
  row goes in your report, not into their row.
- Add a new row for new scope rather than renumbering; run reports cite these
  IDs.
- Deferred work goes to [docs/deferred-work.md](docs/deferred-work.md), which
  tracks consciously postponed items. The status file tracks plan steps only.

A workstream is not finished until its status rows are accurate.

## Starting state and import

The engine was imported from `agent_slope` in M0a; that import is complete and
is recorded in [README.md](README.md). Current position — see the status file
for the per-step detail and evidence:

- **M0a, M0b, M0c: done** (M0b-4 stays `[~]`: clone lowering was proven
  after the IR shape, not prototyped ahead of it). Import, `nsd` rename, D11
  consolidation, versioned BLAKE3, golden digest; IR, both lowerings,
  analyzers retargeted, salvage, neutrality gate incl. the `java-fixture-01`
  strict leg; Orchard grammar swap and the `nsd-v1` freeze.
- **M1–M2: done except M1-8**, which is not started and moved into the M6 run
  by user decision; M1-4 is `[~]` in its row. Snapshots, diffs, identity,
  configuration and base policy trust.
- **M3–M5: done.** All twelve diagnostics, suppressions, V102, exit
  aggregation, the `nsd check` subcommand, and the per-blob cache with LRU
  eviction.
- **M6–M7: in progress.** Check-scope JSON is partly done (M6-1, M6-2); M1-8,
  the scan-scope canonical report, renderers, HTML caps, nested-checkout
  exclusion for `scan`, and the M7-2 delivery sweep remain.
- Everything is merged to `main` (last: PR #6); there is no unmerged branch.
  If this list disagrees with the status file, the status file wins.

The import constraints still bind anything that touches imported material or
reaches back into the archive:

- `~/pet/agent_slope` at `912ec7a1ca2b9e5dd1ffa976a51f98d7d00d6704` is a
  read-only archive. Do not modify it; do not reinitialize or replace this
  repository's Git history. Any further selective import uses `git archive`
  and the allowlist in the implementation plan.
- Keep the two root plan files authoritative; link to them instead of
  creating a divergent `docs/plan.md` copy.
- Never import archived reports, private source, draft plans, build outputs,
  or unrelated untracked files. Preserve existing user changes.

## Implementation boundaries

- Follow M0a -> M0b -> M0c -> M1–M2 -> M3–M5 -> M6–M7. Keep the IR retarget
  separate from the grammar upgrade so measurement changes remain attributable.
- Use a single Rust crate, tree-sitter parsing, Java and JS/TS lowerings,
  normalized IR, and IR-based analyzers. No Python, second parser family,
  frontend plugin system, or watch mode in v1.
- Prototype clone lowering before freezing the IR. Preserve measurement
  algorithms and existing fixture expectations until an explicitly allowed
  re-baseline stage.
- `--staged` reads the index, never worktree source. Checks use trusted base
  policy; candidate configuration cannot weaken its own gate.
- Match underlying findings before applying suppressions. Moving the original
  finding with its unchanged directive is tolerated; transferring a directive
  to an unmatched finding raises S101.
- With V102 set to deny or warn, included unchanged files required for clone
  comparison must also satisfy size, encoding, and capability requirements;
  failures raise A102. Preserve the separate legacy parse-damage tolerance.
- Source comments cannot suppress E101/E102/V102. Trusted policy can downgrade
  them to warn or disable them with off.
- Persist only versioned stable fingerprints. Cache failure must recompute
  without changing verdicts, and staged cache entries must derive from the
  actual analyzed snapshot bytes.

## Verification

- Keep the ten imported test suites green. Run checks appropriate to each
  milestone and add the focused acceptance cases specified in the plans.
- For M0b, run pre-IR and IR binaries against the same fixed fixture checkout,
  with identical target arguments, revision metadata, settings, and legacy JSON
  serialization. Do not compare each crate's own absolute fixture paths.
- Require byte identity on unaffected fixtures. On malformed fixtures, permit
  only explicitly asserted salvage/skip-taxonomy deltas. Delay widened callable
  spans and new report fields until after this gate.
- The grammar milestone must clear all 55 pinned Java varargs failures before
  freezing the profile; explain measurement deltas with hand-checked examples.
- Declare MSRV 1.90, but use the current local toolchain for validation as
  agreed. Do not claim an MSRV compile check was run when it was not.
- Record actual checks and outcomes. Missing private fixtures leave their
  acceptance gates pending; never substitute invented results or silently
  re-baseline an unexplained difference.

## Fixture privacy

Use neutral private fixture labels, language, and verified human/AI/mixed
authorship. The three private fixtures currently have unknown authorship;
do not infer it from complexity scores. Resolve local archived reports by
the revision pins in the final plan. Keep private repository names, checkout
paths, source excerpts, and name mappings outside this repository. Commit only
the specified sanitized numbers-and-hashes digest.

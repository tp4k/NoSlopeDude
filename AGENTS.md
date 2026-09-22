# Working on nsd

## Read first

Read [nsd-plan-final.md](nsd-plan-final.md) for the authoritative v1 contract,
then [nsd-plan-implementation.md](nsd-plan-implementation.md) for execution
order, configuration details, and acceptance checks. The final plan wins
on conflicts. Both include the approved final-review decisions; do not reopen
settled choices without new evidence or a user request.

## Starting state and import

This repository starts with plans and agent instructions only. Implementation
has not started. Begin with M0a / implementation step 1.

- Import selectively from `~/pet/agent_slope` at commit
  `912ec7a1ca2b9e5dd1ffa976a51f98d7d00d6704`, using `git archive` and the
  allowlist in the implementation plan.
- Preserve this repository's plans and instructions. Do not reinitialize or
  replace its Git history. Keep the source repository as a read-only archive.
- Keep the two root plan files authoritative; link to them instead of
  creating a divergent `docs/plan.md` copy.
- Do not import archived reports, private source, draft plans, build outputs,
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

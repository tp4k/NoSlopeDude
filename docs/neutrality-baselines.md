# Updating the neutrality baselines

`tests/neutrality.rs` compares live scans against two committed baselines:

- `tests/golden/neutrality/clean.report.json` — every file under
  `tests/fixtures/` except the malformed fixtures listed in
  `tests/neutrality.rs`;
- `tests/golden/neutrality/malformed.report.json` — those malformed
  fixtures only.

The clean corpus is defined by membership, not by a file list. Adding or
deleting any fixture therefore changes `clean.report.json`, even when no
measurement changed.

`bash scripts/neutrality_gate.sh --capture` rewrites **both** baselines from
whatever is on disk under `tests/fixtures/`, committed or not. It has no
review step of its own. Never hand-edit the JSON; move a baseline only with
this procedure:

1. **Start clean.** `git status --short tests/fixtures` must print nothing
   except the fixture change you mean to capture. An unrelated, uncommitted
   fixture edit would otherwise be baked into the baseline, indistinguishable
   from the intended change.
2. **Prove unchanged fixtures did not move.** Scan the clean corpus without
   the added or changed fixtures and compare it byte-for-byte with the
   currently committed `clean.report.json`; it must be identical. For a
   deletion, scan the corpus *before* deleting (still including the file);
   it must also be identical. Stop on any difference — it is a measurement
   change, not a corpus change, and needs its own explanation.
3. **Capture.** Run `bash scripts/neutrality_gate.sh --capture`.
4. **Bound the diff.** `git diff tests/golden/neutrality` may show only
   what the corpus change predicts: the affected `/scores/*` aggregates and,
   if the added or removed file's callables rank in the top 25, the
   displaced `/top25` rows. `malformed.report.json` must not change unless a
   malformed fixture changed.

The script's default mode (no arguments) compares `HEAD` with the pinned
pre-IR commit `PRE_IR_SHA`. It is **no longer a release gate**: the M0c Java
grammar swap and later measurement changes diverge from that commit by
design, so it reports a divergence on every current tree. Do not re-pin
`PRE_IR_SHA` to make it pass; step 2 above is the drift check.

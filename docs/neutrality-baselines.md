# Updating the neutrality baselines

`tests/neutrality.rs` compares live scans against two committed baselines:

- `tests/golden/neutrality/clean.report.json` — every file under
  `tests/fixtures/` except the malformed fixtures listed in
  `tests/neutrality.rs`;
- `tests/golden/neutrality/malformed.report.json` — those malformed
  fixtures only.

The clean corpus is defined by membership, not by a file list. Adding or
deleting a fixture the scan measures or skips therefore changes
`clean.report.json`, even when no measurement changed; a file the scan
never discovers (e.g. a `.txt`) leaves it unchanged.

`bash scripts/neutrality_gate.sh --capture` rewrites **both** baselines from
whatever is on disk under `tests/fixtures/`, committed or not. It has no
review step of its own. Never hand-edit the JSON; move a baseline only with
this procedure:

1. **Start clean.** `git status --short tests/fixtures` must print nothing.
   An unrelated, uncommitted fixture edit would otherwise be baked into the
   baseline, indistinguishable from the intended change.
2. **Prove the code reproduces the committed baselines.** Before adding,
   editing or deleting any fixture — with `tests/fixtures/` at its committed
   state — run `cargo test --test neutrality`; it must pass. A failure here
   is a measurement change, not a corpus change, and needs its own
   explanation before any capture.
3. **Capture.** Make the fixture change, then run
   `bash scripts/neutrality_gate.sh --capture`.
4. **Explain the diff.** Every pointer `git diff tests/golden/neutrality`
   shows must follow from the fixture change: that file's own `/callables`,
   `/findings` and `/duplicates` entries, its `/skipped_files` row and the
   `/skipped` count if it is skipped, the
   `/scores/*` aggregates it feeds and any `/top25` rows it displaces. A
   change to an unrelated fixture's entries means something else moved —
   stop. `malformed.report.json` must not change unless a malformed fixture
   changed.

The script's default mode (no arguments) compares `HEAD` with the pinned
pre-IR commit `PRE_IR_SHA`. It is **no longer a release gate**: the M0c Java
grammar swap and later measurement changes diverge from that commit by
design, so it reports a divergence on every current tree. Do not re-pin
`PRE_IR_SHA` to make it pass; step 2 above is the drift check.

`--capture` runs the neutrality tests on one thread: another test in the
same binary reads both baselines, and running it in parallel with the
capture tests let it read a half-rewritten file and fail the capture.

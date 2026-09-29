# Finalize and implement `nsd` v1

Revised 2026-09-22 after the implementation review and grilling. Where this
document and `nsd-plan-final.md` disagree, **the final plan wins**. This file
supplies implementation details; every deliberate policy change is recorded
in the final plan's *Amendments* tables (A1–A10).

## Summary

- `nsd-plan-final.md` has been revised in place: corrections #15–#18 (the four
  recoverable internal inconsistencies; a fifth claimed by the earlier draft
  could not be reconstructed and is not claimed), the final-review neutrality
  harness and `warn|off` corrections, and amendments A1–A10.
- The handoff creates `~/pet/nsd` with Git, both final plans, `AGENTS.md`,
  and `CLAUDE.md`. Implementation begins by importing the engine into that
  existing repository from
  **`agent_slope@912ec7a1ca2b9e5dd1ffa976a51f98d7d00d6704`** (branch
  `docs/close-scanner-run-ledger`), which tracks the deferred-work ledger and
  both perf rows. Leave `agent_slope` untouched as the archive.
- Import via `git archive` (one fixture is literally named
  `"><img onerror=1>.js`; shell globbing mangles it): `src/`, `tests/`, `docs/`
  including `docs/deferred-work.md`, `scripts/`, `Cargo.toml`, `Cargo.lock`,
  `.gitignore`. Keep `nsd-plan-final.md` and `nsd-plan-implementation.md` at
  the repository root as the authoritative plans; link to them from docs
  instead of maintaining a second copy of the final plan.
- **Do not import the archived Java fixture report.** It holds 2.6 MB of private
  source excerpts and the absolute path of the work checkout. Commit a
  numbers-and-hashes **digest** instead (A1): per-language erosion and
  verbosity, findings per rule ID, clone group count and total redundant lines,
  skips per reason, top-25 as `(cc, sloc, mass)` triples without names or
  paths, and a BLAKE3 of the excerpt-stripped, path-relativised body. Private
  repository names must not appear in the new repo; identify fixtures
  only by neutral label, language, and verified authorship (human, AI, mixed,
  or unknown).
- Private fixture labels and revision pins are in the final plan's measured
  starting state. Resolve local archived reports by `scan.revision.sha`;
  authorship is currently unknown for all three. Keep private checkout paths
  and repository-name mappings local, outside the new repository.
- Exclude draft plans, HTML reports, private JS/TS fixture outputs, `target/`,
  and every other untracked source-archive file.
- Record provenance to the SHA above in the new README. `agent_slope` has no
  attribution files to preserve.

## Public contracts

- CLI:
  - `nsd scan <target> --output <dir> [--include-tests] [--exclude …] [--min-clone-lines <n>]`
  - `nsd check --staged [--config <path>] [--format terminal|json|agent] [--allow-new-suppressions]`
  - `nsd check --base <ref> [--worktree] [--config <path>] [--format …] [--allow-new-suppressions]`
  - `--staged` and `--base` are mutually exclusive; `--worktree` requires `--base`; terminal is the default format.
  - `--min-clone-lines` defaults to `measurement.min_clone_lines` from the repository-root `nsd.yml` when present, else `10`.
  - In an unborn repository, `--staged` compares the index with Git's empty tree and uses built-in policy.
- `nsd.yml` is strict and repository-root scoped:

  ```yaml
  version: 1
  include: [src/**]       # omitted means every supported source path
  exclude: [vendor/**]

  measurement:
    min_clone_lines: 10   # any positive integer; enters the fingerprint and cache key (A2)

  policy:
    NSD-E101: deny
    NSD-E102: deny
    NSD-V101: deny
    NSD-V102: deny
    NSD-S102: warn

  output:
    max_terminal_diagnostics: 50
    max_agent_diagnostics: 30
  ```

  Unknown/duplicate fields, unsupported versions or codes, invalid globs, invalid severity values, and a non-positive `min_clone_lines` produce `C102`. Include narrows scope; immutable built-in exclusions (dependency, build, generated, minified, WebJar paths, and **nested `.git` checkouts**) and `exclude` then subtract from it. `check` has no default test exclusion. Negated/re-inclusion patterns are rejected.
- A trusted `--config` completely replaces repository policy for that invocation. Otherwise checks use base-snapshot config; candidate config is validated and reported through `C101` but cannot affect its own check.
- Canonical JSON uses `result_scope: full` for `scan` and `result_scope: changed` for `check`. Check output includes repository summaries, changed entities, relevant base matches, and the complete uncapped diagnostic set. Agent output contains capped violations only.
- Serialize floats using deterministic shortest round-trip representation and hashes as algorithm-prefixed lowercase hex.

## Policy and analysis behavior

- Fix the source-file ceiling at 1 MiB:
  - Exactly 1 MiB is accepted.
  - An included changed file above it produces `A102`.
  - When `V102` is enabled (`deny` or `warn`), an included unchanged file
    required for clone comparison that exceeds the limit also produces `A102`.
  - `scan` records `file_too_large` as an analysis failure, sets `incomplete`, and retains scan's non-policy exit behavior.
  - The limit is immutable; trusted path exclusion is the only escape.
- Non-UTF-8 included source paths produce `A102` during checks and are rendered with deterministic percent escaping. Invalid source encoding also produces `A102`. Scan records either as an analysis failure rather than silently dropping it.
- Required clone-index coverage includes unchanged files while `V102` is
  enabled: size, encoding, or unavailable analyzer capability must produce
  `A102`, never a silent omission and pass (A10). Trusted exclusion removes
  the file from scope. `V102: off` removes this additional unchanged-file
  obligation, not analysis requirements for changed files. The separate
  tolerance for mapped legacy parse damage remains unchanged.
- Skip symlinks and parent-repository submodule entries with explicit reasons. Unsupported binary files are skipped; a changed supported-extension file with invalid text encoding fails closed.
- Git rename detection uses libgit2 with a fixed 50% similarity threshold. Tests assert the configured threshold plus one clear rename and one clear non-rename fixture; exact 49%/50% fixtures are not constructible because libgit2 scores hashed content chunks (A6).
- `V102` behavior (A3):
  - Reduce to maximal clone groups first and emit one diagnostic per regressing candidate occurrence.
  - **Move mapping runs first.** A base occurrence deleted by the diff maps to an added candidate occurrence when the base occurrence's normalized token stream appears as one contiguous run inside the added stream and the added occurrence's extra executable lines are within `max(1, floor(base_occurrence_lines / 10))`. Repeated identical occurrences pair *k*-th to *k*-th in source order. Mapped pairs are moves and never fire.
  - A new occurrence is an added occurrence with no diff/rename/move-mapped base occurrence that has an identical qualifying occurrence elsewhere in the candidate snapshot.
  - An extension regresses when added duplicated executable lines are strictly greater than `max(1, floor(base_occurrence_lines / 10))`.
  - Ignore pure line shifts; exclude the replaced base version of a modified path.
- `S102` is delta-based during `check`: report new/modified invalid or unused directives, plus previously valid directives made unused by the candidate change. Matched legacy defects are tolerated. A one-snapshot `scan` reports all invalid/unused directives.
- Match underlying findings before applying suppressions. Suppression identity
  includes the rule ID and its matched finding: moving the original finding
  with its unchanged directive is tolerated; transferring that directive to
  an unmatched finding raises `S101` (A9).
- `--allow-new-suppressions` makes `S101` exit-neutral but keeps it in output; it does not waive `E101`, `E102`, or `V102`. Those diagnostics can only be changed by trusted base policy.
- Trusted policy may downgrade `E101`, `E102`, and `V102` to exit-neutral
  `warn` or disable them with `off`; neither is a source suppression.
- Preserve the source specification's parse-damage rule: tolerate damage only when it maps through unchanged source; changed-line intersection, unmappable damage, reduced coverage, or an unmeasured changed entity produces `A101`.

## Implementation sequence

1. **Bootstrap and measurement isolation**
   - Use the initialized handoff repository; preserve its plans and agent instructions. Create the selective fresh import (`git archive` from `912ec7a`), the provenance record, and the golden digest (A1). Declare `rust-version = "1.90"` (A4).
   - Consolidate executable-line logic and replace persisted `DefaultHasher` output with versioned BLAKE3. `report.json` carries no persisted fingerprint, so this cannot disturb the neutrality gate.
   - Introduce the normalized IR and Java/JS-TS lowerings against existing grammars; prototype clone-token lowering first.
   - Require byte-identical reports on unaffected fixtures and on `java-fixture-01`, compared locally against the archived full `report.json` from the checkout path it records. Do **not** clear the `Callable::end_line` ledger row before this gate passes; it widens Top-25 spans and would fail the gate by design. For existing malformed fixtures, allow only explicitly asserted salvage/skip-taxonomy deltas.
   - Run the pre-IR and IR binaries against the same fixed fixture checkout
     with identical target arguments, revision SHA, dirty state, and settings.
     Comparing each crate's `CARGO_MANIFEST_DIR` fixture paths is invalid.
     Keep legacy JSON serialization through M0b; new report fields and widened
     spans land afterward. Unaffected measurements remain identical even in
     fixtures with explicitly permitted salvage/skip-taxonomy deltas.
2. **Grammar and profile freeze**
   - Move to Orchard 0.5.18 and tree-sitter 0.27.0; retain current JavaScript/TypeScript grammar versions. A probe crate has already shown the four compile together and that Orchard parses `Class<?> @Nullable ... cs` cleanly (A7); the 55-file Spring corpus run remains the gate.
   - Re-run and explain the Spring/Angular deltas; stop if all 55 Java varargs failures do not clear.
   - Freeze `nsd-v1`. Its fingerprint includes the IR version, both lowering versions, the tree-sitter runtime version, the three grammar versions (Orchard, JavaScript, TypeScript), the rule catalog, and the effective clone configuration including `min_clone_lines`.
   - Clear the remaining cheap ledger rows here, `end_line` included.
3. **Snapshots, identity, and configuration**
   - Add commit/index/worktree snapshots, empty-tree staging support, central diff mapping, 50% rename detection, strict YAML parsing including `measurement.min_clone_lines`, base-policy trust, callable matching, and deterministic snapshot IDs.
   - Use maintained `serde_yaml_ng` rather than deprecated `serde_yaml`; add `git2` and BLAKE3 as separate runtime dependencies (three new runtime dependencies in total).
   - **Parallel track (user-approved 2026-09-23).** The measurement-free part of this step starts early on `feat/m1-snapshots`, branched from `c5267c0` while M0b/M0c continue on `feat/m0a-import`: commit/index/worktree snapshots, empty-tree staging, central diff/line mapping, 50% rename detection, merge-base and `G101`, strict `nsd.yml` parsing with `C102` (`min_clone_lines` parsed but not yet in the fingerprint), and Git-backed discovery with immutable exclusions. It adds new modules only and does not wire them into `pipeline.rs` or touch analyzers. It does not change measurement, so M0's attribution order holds. Line-independent callable identity, callable matching, coverage metadata, and fingerprint wiring wait for the `nsd-v1` freeze. Rebase onto the M0 branch once M0c lands.
4. **Policy, suppressions, clones, and cache**
   - Implement all twelve diagnostics, delta-based finding/suppression matching, V102 move mapping and material clone-extension logic, and exit aggregation.
   - Store versioned cache data under `$GIT_COMMON_DIR/nsd/cache/v1`, keyed by blob OID, language/grammar, and measurement fingerprint. Writes are atomic; corruption or I/O failure remains a recomputable miss; concurrent writers never corrupt a reader.
   - **Best-effort LRU eviction, 1 GiB / 30-day default.** The payload is large because `enumerate_candidates` emits every contiguous sub-run, so eviction is real work, not a follow-up.
5. **Output and delivery**
   - Implement canonical scoped JSON, terminal and agent renderers, scan-only HTML with excerpts capped at 20 lines / 4 KiB and an explicit truncation marker, deterministic caps, and per-reason analysis accounting.
   - Nested-checkout exclusion in `scan` discovery: a directory containing its own `.git` is excluded regardless of `.gitignore`.
   - Document copyable pre-commit and CI commands; do not add a hook installer or provider-specific workflow in v1.

## Test and acceptance plan

- Keep all ten existing suites green and add IR neutrality, lowering isolation, cross-language metric parity, damage-span, and fingerprint tests.
- Add boundary coverage for:
  - empty-tree first commit;
  - rename detection: configured threshold asserted, one clear rename, one clear non-rename (A6);
  - 1 MiB/exactly-over-1-MiB files;
  - non-UTF-8 paths and source;
  - unchanged included clone inputs failing size, encoding, or capability
    requirements produce `A102` under `V102: deny` and `warn`; trusted
    exclusion removes the obligation, and `off` removes only the additional
    unchanged-file obligation; mapped legacy parse damage stays tolerated;
  - clone extensions at base `10`: `+1` passes, `+2` fails; base `100`: `+10` passes, `+11` fails;
  - clone moves: a block moved across files passes, the same block copied across files fails, a moved block edited inside its body fails (A3);
  - `min_clone_lines` from config changes the measurement fingerprint and the cache key; `0` and negative values produce `C102`; the `scan` flag overrides the config value;
  - strict YAML validation, immutable exclusions, nested `.git` checkout excluded even when not gitignored, external config replacement, and candidate config weakening;
  - regression-only `S102` and exit-neutral allowed `S101`;
  - moving an original finding with its unchanged suppression passes;
    transferring an existing directive to an unmatched finding raises `S101`;
  - full-versus-changed JSON scopes and uncapped JSON diagnostics.
- Cache: cold/warm equivalence, wrong-language same-blob isolation, corruption recovery, concurrent writers, eviction by size and by age, and that no persisted key derives from `DefaultHasher`.
- Prove exits `0/1/2/3`, unchanged staged verdicts under unstaged edits, and byte-stable JSON across roots, thread counts, and cache states.
- Run tests with the current Homebrew toolchain only. MSRV 1.90 is declared, not exercised (A4).
- Re-run pinned `java-fixture-01`, `js-ts-fixture-01`, Spring, and Angular acceptance scans locally; the `java-fixture-01` digest is the committed golden, the archived full report is the local byte-identity reference, and the dirty `js-ts-fixture-02` result is never adopted.

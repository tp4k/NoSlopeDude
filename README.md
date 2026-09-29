# nsd

## Provenance

This repository's engine (`src/`, `tests/`, `docs/`, `scripts/`,
`Cargo.toml`, `Cargo.lock`, `.gitignore`) was imported selectively from
`agent_slope` at commit `912ec7a1ca2b9e5dd1ffa976a51f98d7d00d6704`
(branch `docs/close-scanner-run-ledger`), which tracks the deferred-work
ledger and both perf rows in `docs/measurements.md`. The import used
`git archive` against that pin, with no path filtering, and the crate,
binary, CLI name, HTML report title, and docs/scripts were then renamed
from `agent_slope` to `nsd`.

The source repository (`agent_slope`) is kept as a read-only archive and
was not modified as part of this import.

## Spec

The authoritative specification for this project is
[`nsd-plan-final.md`](nsd-plan-final.md) and
[`nsd-plan-implementation.md`](nsd-plan-implementation.md) at the
repository root. This README does not restate their contents.

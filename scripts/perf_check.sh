#!/usr/bin/env bash
# Scaffold (WS-3): measures cold and warm `nsd check` time and the cache
# size on the perf fixture and records one row under `## Check rows` in
# docs/measurements.md. Every function below is a placeholder until the
# implementation lands.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=perf_scan.sh
source "$SCRIPT_DIR/perf_scan.sh"

not_implemented() {
  echo "error: not implemented" >&2
  return 1
}

# build_check_repo <fixture root> <repo dir>
build_check_repo() { not_implemented; }

# cache_size_mb <repo dir>
cache_size_mb() { not_implemented; }

# require_matching_outputs <cold json> <warm json>
require_matching_outputs() { not_implemented; }

# main [fixture root] [nsd binary]
main() {
  not_implemented
  exit 1
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi

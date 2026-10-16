#!/usr/bin/env bash
# WS-3: runs `nsd check --base HEAD~1 --format json` twice (cold, then warm
# cache) under `/usr/bin/time -l` on a scratch git repository built from a
# plain copy of the D25 perf fixture plus one small new Java file, and
# inserts a row (fixture shas, cold and warm wall seconds and peak RSS, the
# cache size after the warm run, the cold run's check exit status, date+HEAD sha,
# machine) under `## Check rows` in docs/measurements.md. Refuses to run
# without the fixture (see scripts/fetch_perf_fixture.sh) and refuses the
# row when either run did not complete (process status outside 0-3, or stdout
# not one check JSON document whose exit_status equals that status) or when
# the cold and warm outputs differ. The check runs in the scratch
# repository, never inside the fixture clones, so their `.git` stays
# unchanged. Sources scripts/perf_scan.sh for build_plain_tree, date_cell,
# make_temp_paths and insert_table_row; its `main` is guarded, so sourcing
# only defines functions. `tests/perf_check.rs` sources this file too.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=perf_scan.sh
source "$SCRIPT_DIR/perf_scan.sh"

CHECK_TABLE_HEADING="## Check rows"
CANDIDATE_FILE="nsd-perf-candidate/src/main/java/nsdperf/PerfCandidate.java"
KIB_PER_MIB=1024

# git_scratch <repo dir> <git args...>
# Runs git with identity, hooks, signing, global excludes and gc fixed per
# command, so no global git config is read for those or changed.
git_scratch() {
  local repo="$1"
  shift
  git -C "$repo" \
    -c user.name=nsd-perf -c user.email=nsd-perf@example.invalid \
    -c commit.gpgsign=false -c core.hooksPath=/dev/null \
    -c core.excludesFile=/dev/null -c gc.auto=0 "$@"
}

# build_check_repo <fixture root> <repo dir>
# Builds the plain tree of the fixture clones in <repo dir>, `git init`s it,
# commits it as the base, then commits one small deterministic new Java file
# as the candidate. The fixture root is only read.
build_check_repo() {
  local fixture_root="$1" repo="$2"
  build_plain_tree "$fixture_root" "$repo" || return 1
  git_scratch "$repo" init --quiet || return 1
  git_scratch "$repo" add -A || return 1
  git_scratch "$repo" commit --quiet -m "base" || return 1
  mkdir -p "$repo/$(dirname "$CANDIDATE_FILE")" || return 1
  cat >"$repo/$CANDIDATE_FILE" <<'JAVA' || return 1
package nsdperf;

public class PerfCandidate {
    public int classify(int value) {
        if (value < 0) {
            return -1;
        }
        int total = 0;
        for (int i = 0; i < value; i++) {
            if (i % 2 == 0) {
                total += i;
            } else {
                total -= 1;
            }
        }
        return total;
    }
}
JAVA
  git_scratch "$repo" add -A || return 1
  git_scratch "$repo" commit --quiet -m "candidate" || return 1
}

# cache_size_mb <repo dir>
# Prints the size of `<git dir>/nsd/cache` in MB (two decimals), from
# `du -sk` (kilobyte blocks, so divided by 1024).
cache_size_mb() {
  local repo="$1" git_dir kb
  git_dir="$(git -C "$repo" rev-parse --absolute-git-dir)" || return 1
  if [ ! -d "$git_dir/nsd/cache" ]; then
    echo "error: no cache directory at $git_dir/nsd/cache" >&2
    return 1
  fi
  kb="$(du -sk "$git_dir/nsd/cache")" || return 1
  kb="${kb%%[[:space:]]*}"
  awk -v kb="$kb" -v kib="$KIB_PER_MIB" 'BEGIN { printf "%.2f", kb / kib }'
}

# require_matching_outputs <cold json> <warm json>
# Exits the caller's flow with an `error:` line when the two check outputs
# differ: a row measured on diverging results compares different work.
require_matching_outputs() {
  if ! cmp -s "$1" "$2"; then
    echo "error: cold and warm check outputs differ ($1 vs $2); not recording a row" >&2
    return 1
  fi
}

# require_completed_check <run name> <status> <json file>
# Refuses (an `error:` line, return 1) unless the run is a completed check:
# <status> is 0-3 (docs/cli.md exit codes) and <json file> holds exactly one
# JSON document with result_scope "check", schema_version 1 and an
# exit_status equal to <status>. An empty output or a crash (e.g. 101, 125)
# is therefore never recorded.
require_completed_check() {
  local name="$1" status="$2" json="$3"
  if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required to validate the $name check output; not recording a row" >&2
    return 1
  fi
  case "$status" in
    0 | 1 | 2 | 3) ;;
    *)
      echo "error: $name check exited $status, not a check status 0-3; not recording a row" >&2
      return 1
      ;;
  esac
  if ! jq -e -s --argjson status "$status" \
    'length == 1 and (.[0] | type == "object" and .result_scope == "check" and .schema_version == 1 and .exit_status == $status)' \
    "$json" >/dev/null 2>&1; then
    echo "error: $name check output ($json) is not one check JSON document with exit_status $status; not recording a row" >&2
    return 1
  fi
}

# run_timed_check <nsd binary> <repo dir> <json out> <time log>
# Prints the check's exit status; the caller validates it with
# require_completed_check (a failing check, exit 1-3, is still a measured run).
run_timed_check() {
  local bin="$1" repo="$2" json="$3" log="$4" status=0
  (cd "$repo" && /usr/bin/time -l "$bin" check --base HEAD~1 --format json >"$json" 2>"$log") || status=$?
  printf '%s' "$status"
}

# time_cells <time log>
# Prints `<wall seconds> <peak RSS in MB>` from `/usr/bin/time -l` output.
time_cells() {
  local wall rss_bytes
  wall="$(grep ' real' "$1" | awk '{print $1}')"
  rss_bytes="$(grep 'maximum resident set size' "$1" | awk '{print $1}')"
  awk -v wall="$wall" -v bytes="$rss_bytes" 'BEGIN { printf "%s %.2f", wall, bytes / 1048576 }'
}

main() {
  local fixture_root="${1:-${TMPDIR:-/tmp}/nsd-perf-fixture}"
  local bin="${2:-}"
  local spring_dir="$fixture_root/spring-framework"
  local angular_dir="$fixture_root/angular"

  if [ ! -d "$spring_dir/.git" ] || [ ! -d "$angular_dir/.git" ]; then
    echo "error: fixture root $fixture_root is missing spring-framework or angular; run scripts/fetch_perf_fixture.sh first" >&2
    exit 1
  fi

  local spring_sha angular_sha
  spring_sha="$(git -C "$spring_dir" rev-parse HEAD)"
  angular_sha="$(git -C "$angular_dir" rev-parse HEAD)"

  if [ -z "$bin" ]; then
    (cd "$REPO_ROOT" && cargo build --release)
    bin="$REPO_ROOT/target/release/nsd"
  fi

  make_temp_paths
  build_check_repo "$fixture_root" "$plain_dir"

  # Cold means no cache: drop whatever the repo's git dir already holds.
  local git_dir
  git_dir="$(git -C "$plain_dir" rev-parse --absolute-git-dir)"
  rm -rf "$git_dir/nsd"

  local cold_json="$output_dir/cold.json" warm_json="$output_dir/warm.json"
  local cold_log="$output_dir/cold.time" warm_log="$output_dir/warm.time"
  local cold_status warm_status
  cold_status="$(run_timed_check "$bin" "$plain_dir" "$cold_json" "$cold_log")"
  warm_status="$(run_timed_check "$bin" "$plain_dir" "$warm_json" "$warm_log")"

  require_completed_check cold "$cold_status" "$cold_json"
  require_completed_check warm "$warm_status" "$warm_json"
  require_matching_outputs "$cold_json" "$warm_json"

  local cold_cells warm_cells cache_mb
  cold_cells="$(time_cells "$cold_log")"
  warm_cells="$(time_cells "$warm_log")"
  cache_mb="$(cache_size_mb "$plain_dir")"

  local date_cell_value
  date_cell_value="$(date_cell)"

  local row
  row="| $date_cell_value | $(uname -srm) | spring-framework@$spring_sha | angular@$angular_sha | 1 new .java file | ${cold_cells%% *}s | ${cold_cells##* } MB | ${warm_cells%% *}s | ${warm_cells##* } MB | ${cache_mb} MB | $cold_status |"

  insert_table_row "$MEASUREMENTS_FILE" "$CHECK_TABLE_HEADING" "$row"
  echo "$row"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi

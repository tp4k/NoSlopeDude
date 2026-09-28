#!/usr/bin/env bash
# WS-6/D24 (hardened by WS-8's B10-B12): runs exactly one scan of the D25
# perf fixture under `/usr/bin/time -l` and appends a row (fixture shas,
# the scanner's own scanned-source-line count, wall-clock seconds, peak RSS
# in MB, date+HEAD sha, machine) to docs/measurements.md. Refuses to run if
# the fixture (see scripts/fetch_perf_fixture.sh) is not present, rather
# than measuring an empty tree.
#
# `report.json` parsing lives in plain functions below, not inlined into
# `main`, so `tests/perf_scan.rs` can `source` this file and call them
# directly against synthetic report.json text; the `[[ "${BASH_SOURCE[0]}"
# == "$0" ]]` guard at the bottom keeps `main` from running when sourced.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MEASUREMENTS_FILE="$REPO_ROOT/docs/measurements.md"

# extract_overall_block <report.json path>
# Prints the `"overall": { ... }` block (brace-depth counted, so it does
# not depend on key order inside that block), or nothing if that anchor is
# absent.
extract_overall_block() {
  local file="$1"
  awk '
    /"overall": \{/ { capture = 1; depth = 0 }
    capture {
      depth += gsub(/\{/, "{")
      depth -= gsub(/\}/, "}")
      print
      if (depth == 0) { exit }
    }
  ' "$file"
}

# extract_scanned_lines <report.json path>
# Prints the `overall` block's own `scanned_lines` value. B10:
# `scanned_lines` also appears inside the per-family (java/js_ts) score
# blocks, so a plain top-level grep can silently read the wrong one if
# `"overall": {` is ever absent -- e.g. a key rename in `report.json`.
# Exits 1 with an `error:` line instead of falling back to that plain
# grep.
extract_scanned_lines() {
  local file="$1"
  local overall_block
  overall_block="$(extract_overall_block "$file")"
  if [ -z "$overall_block" ]; then
    echo "error: \"overall\": { anchor not found in $file; refusing to fall back to a plain top-level grep" >&2
    return 1
  fi
  local scanned_lines
  scanned_lines="$(printf '%s\n' "$overall_block" | grep -m1 '"scanned_lines"' | grep -o '[0-9]\+' || true)"
  if [ -z "$scanned_lines" ] || [ "$scanned_lines" -eq 0 ]; then
    echo "error: could not read a nonzero scanned_lines count from the overall block in $file" >&2
    return 1
  fi
  printf '%s' "$scanned_lines"
}

# extract_incomplete <report.json path>
# Prints the top-level `incomplete` boolean. It is a unique top-level key,
# so a plain grep is sound (unlike `scanned_lines`/`skipped_files`, which
# both recur inside nested blocks).
extract_incomplete() {
  local file="$1"
  grep -m1 '"incomplete"' "$file" | grep -o 'true\|false'
}

# extract_skipped_block <report.json path>
# Prints the `"skipped_files": [ ... ]` array. B11: depth is counted on a
# copy of each line with JSON string *contents* blanked out first, so a
# skipped path containing a literal `]` (or `[`) can never be mistaken for
# array structure and truncate the block early. The printed block is the
# original, unblanked text, so a downstream key-name grep still works.
extract_skipped_block() {
  local file="$1"
  awk '
    function blank_strings(s,    i, c, out, in_str) {
      out = ""
      in_str = 0
      for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (in_str && c == "\\") { i++; continue }
        if (c == "\"") { in_str = !in_str; continue }
        if (in_str) { continue }
        out = out c
      }
      return out
    }
    /"skipped_files": \[/ { capture = 1; depth = 0 }
    capture {
      safe = blank_strings($0)
      depth += gsub(/\[/, "[", safe)
      depth -= gsub(/\]/, "]", safe)
      print
      if (depth == 0) { exit }
    }
  ' "$file"
}

# extract_skipped_count <report.json path>
# Prints the number of `skipped_files` entries (each carries a unique
# `"relative_path"` key), read from the block above.
extract_skipped_count() {
  local file="$1"
  extract_skipped_block "$file" | grep -c '"relative_path"' || true
}

# head_sha_annotation
# Prints `(nsd@<40-hex HEAD sha>)`, with a `+dirty` suffix appended to the
# sha when `git status --porcelain` (confirmed via `git status --help`)
# reports any change under the paths that actually produce the scanned
# binary. B12: this is how the HEAD sha reaches the appended row.
head_sha_annotation() {
  local sha dirty
  sha="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  dirty=""
  if [ -n "$(git -C "$REPO_ROOT" status --porcelain -- src Cargo.toml Cargo.lock)" ]; then
    dirty="+dirty"
  fi
  printf '(nsd@%s%s)' "$sha" "$dirty"
}

# date_cell
# Prints the row's date cell: the UTC date plus the HEAD sha annotation
# above, following the same *placement* as the hand-annotated rows in
# docs/measurements.md (a parenthetical in the date cell).
date_cell() {
  printf '%s %s' "$(date -u +%Y-%m-%d)" "$(head_sha_annotation)"
}

main() {
  local fixture_root="${1:-${TMPDIR:-/tmp}/nsd-perf-fixture}"
  local spring_dir="$fixture_root/spring-framework"
  local angular_dir="$fixture_root/angular"

  if [ ! -d "$spring_dir/.git" ] || [ ! -d "$angular_dir/.git" ]; then
    echo "error: fixture root $fixture_root is missing spring-framework or angular; run scripts/fetch_perf_fixture.sh first" >&2
    exit 1
  fi

  local spring_sha angular_sha
  spring_sha="$(git -C "$spring_dir" rev-parse HEAD)"
  angular_sha="$(git -C "$angular_dir" rev-parse HEAD)"

  (cd "$REPO_ROOT" && cargo build --release)
  local bin="$REPO_ROOT/target/release/nsd"

  # Not `local`: the EXIT trap below fires after main() returns, once the
  # script reaches its own end, outside main's dynamic scope -- a `local`
  # binding here would already be gone by then (`set -u` catches exactly
  # that as "unbound variable").
  output_dir="$(mktemp -d)"
  time_log="$(mktemp)"
  trap 'rm -rf "$output_dir" "$time_log"' EXIT

  /usr/bin/time -l "$bin" scan "$fixture_root" --output "$output_dir" >/dev/null 2>"$time_log"

  local wall_seconds peak_rss_bytes peak_rss_mb
  wall_seconds="$(grep ' real' "$time_log" | awk '{print $1}')"
  peak_rss_bytes="$(grep 'maximum resident set size' "$time_log" | awk '{print $1}')"
  peak_rss_mb="$(awk -v bytes="$peak_rss_bytes" 'BEGIN { printf "%.2f", bytes / 1048576 }')"

  local report_json="$output_dir/report.json"
  local scanned_lines incomplete skipped_count
  scanned_lines="$(extract_scanned_lines "$report_json")"
  incomplete="$(extract_incomplete "$report_json")"
  skipped_count="$(extract_skipped_count "$report_json")"

  local row
  row="| $(date_cell) | $(uname -srm) | spring-framework@$spring_sha | angular@$angular_sha | $scanned_lines | ${wall_seconds}s | ${peak_rss_mb} MB | $incomplete | $skipped_count |"

  echo "$row" >>"$MEASUREMENTS_FILE"
  echo "$row"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi

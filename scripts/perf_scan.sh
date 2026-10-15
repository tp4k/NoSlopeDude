#!/usr/bin/env bash
# WS-6/D24 (hardened by WS-8's B10-B12): runs exactly one scan of the D25
# perf fixture under `/usr/bin/time -l` and appends a row (fixture shas,
# the scanner's own scanned-source-line count, wall-clock seconds, peak RSS
# in MB, date+HEAD sha, machine) to docs/measurements.md. Refuses to run if
# the fixture (see scripts/fetch_perf_fixture.sh) is not present, rather
# than measuring an empty tree. The scan runs on a plain copy of both clones
# without their `.git` entries (see build_plain_tree).
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
# Prints the top-level `incomplete` boolean. The match is anchored to the
# two-space indent of a top-level key in the pretty-printed report, so a
# skipped path whose value is `"incomplete"` cannot be mistaken for it.
extract_incomplete() {
  local file="$1"
  grep -m1 '^  "incomplete"' "$file" | grep -o 'true\|false'
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
  local sha dirty status
  sha="$(git -C "$REPO_ROOT" rev-parse HEAD)" || return 1
  status="$(git -C "$REPO_ROOT" status --porcelain -- src Cargo.toml Cargo.lock)" || return 1
  dirty=""
  if [ -n "$status" ]; then
    dirty="+dirty"
  fi
  printf '(nsd@%s%s)' "$sha" "$dirty"
}

# date_cell
# Prints the row's date cell: the UTC date plus the HEAD sha annotation
# above, following the same *placement* as the hand-annotated rows in
# docs/measurements.md (a parenthetical in the date cell). B12/row 7: a
# failing git inside head_sha_annotation must fail this function closed
# too, not print a truncated cell -- `set -e` is not inherited by a
# command substitution, so the failure (of `date -u` as well) is captured
# explicitly here rather than relying on `$(head_sha_annotation)`
# propagating it.
date_cell() {
  local ann day
  ann="$(head_sha_annotation)" || return 1
  day="$(date -u +%Y-%m-%d)" || return 1
  printf '%s %s' "$day" "$ann"
}

# build_plain_tree <fixture root> <dest dir>
# Copies the working tree of each fixture clone (spring-framework, angular)
# into <dest dir>/<clone>, leaving out only that clone's top-level `.git`
# entry. `nsd scan` treats a directory that holds its own `.git` as a nested
# checkout and skips it (M7-1), so scanning the clones in place would read 0
# lines; a plain tree keeps one scan and a file set comparable with earlier
# measurements rows. `git archive` is not used (it honours `export-ignore`
# and would drop files) and neither is `git worktree` (its `.git` file is
# itself a nested-checkout marker). Nothing under the fixture root is changed.
build_plain_tree() {
  local fixture_root="$1" dest="$2" clone entry
  for clone in spring-framework angular; do
    mkdir -p "$dest/$clone" || return 1
    for entry in "$fixture_root/$clone"/* "$fixture_root/$clone"/.[!.]* "$fixture_root/$clone"/..?*; do
      [ -e "$entry" ] || [ -L "$entry" ] || continue
      [ "$(basename "$entry")" = ".git" ] && continue
      cp -RP "$entry" "$dest/$clone/" || return 1
    done
  done
}

# insert_table_row <file> <heading> <row>
# Inserts <row> right after the last row of the first table under the line
# that is exactly <heading>. Several script-written tables share one file,
# so appending at EOF would put the row under whichever table is last. It
# fails closed (`error:` on stderr, exit 1, <file> untouched) when the
# heading is absent or its section holds no table before the next `## `
# heading, instead of falling back to EOF.
insert_table_row() {
  local file="$1" heading="$2" row="$3" tmp="$1.tmp.$$"
  if ! ROW="$row" awk -v heading="$heading" '
    function emit() { print ENVIRON["ROW"]; state = 3 }
    state == 0 && $0 == heading { state = 1; print; next }
    state == 1 && /^## / { state = -1 }
    state == 1 && /^\|/ { state = 2 }
    state == 2 && !/^\|/ { emit() }
    { print }
    END { if (state == 2) emit(); exit (state == 3 ? 0 : 1) }
  ' "$file" >"$tmp"; then
    rm -f "$tmp"
    echo "error: no table under the heading \"$heading\" in $file; not inserting" >&2
    return 1
  fi
  cat "$tmp" >"$file" && rm -f "$tmp"
}

# make_temp_paths
# Creates the scan's three temp paths in the globals `output_dir`,
# `time_log` and `plain_dir` and installs the EXIT trap that removes them.
# The trap is installed before the first mktemp, so a later failing mktemp
# does not leak the earlier paths.
# Not `local`: the trap fires after main() returns, outside main's dynamic
# scope, where a `local` binding is already gone (`set -u` reports it as
# "unbound variable").
make_temp_paths() {
  output_dir=""
  time_log=""
  plain_dir=""
  trap 'rm -rf "$output_dir" "$time_log" "$plain_dir"' EXIT
  output_dir="$(mktemp -d)"
  time_log="$(mktemp)"
  plain_dir="$(mktemp -d "${TMPDIR:-/tmp}/nsd-perf-plain.XXXXXX")"
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

  make_temp_paths

  build_plain_tree "$fixture_root" "$plain_dir"

  /usr/bin/time -l "$bin" scan "$plain_dir" --output "$output_dir" >/dev/null 2>"$time_log"

  local wall_seconds peak_rss_bytes peak_rss_mb
  wall_seconds="$(grep ' real' "$time_log" | awk '{print $1}')"
  peak_rss_bytes="$(grep 'maximum resident set size' "$time_log" | awk '{print $1}')"
  peak_rss_mb="$(awk -v bytes="$peak_rss_bytes" 'BEGIN { printf "%.2f", bytes / 1048576 }')"

  local report_json="$output_dir/report.json"
  local scanned_lines incomplete skipped_count
  scanned_lines="$(extract_scanned_lines "$report_json")"
  incomplete="$(extract_incomplete "$report_json")"
  skipped_count="$(extract_skipped_count "$report_json")"

  # date_cell's own exit status must gate the row build directly: inside
  # the interpolated string below it would be masked by uname's (the
  # last substitution in that assignment), which is why it is computed
  # here, on its own, first.
  local date_cell_value
  date_cell_value="$(date_cell)"

  local row
  row="| $date_cell_value | $(uname -srm) | spring-framework@$spring_sha | angular@$angular_sha | $scanned_lines | ${wall_seconds}s | ${peak_rss_mb} MB | $incomplete | $skipped_count |"

  insert_table_row "$MEASUREMENTS_FILE" "## Rows" "$row"
  echo "$row"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi

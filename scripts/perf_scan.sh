#!/usr/bin/env bash
# WS-6/D24: runs exactly one scan of the D25 perf fixture under
# `/usr/bin/time -l` and appends a row (fixture shas, the scanner's own
# scanned-source-line count, wall-clock seconds, peak RSS in MB, date and
# machine) to docs/measurements.md. Refuses to run if the fixture (see
# scripts/fetch_perf_fixture.sh) is not present, rather than measuring an
# empty tree.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MEASUREMENTS_FILE="$REPO_ROOT/docs/measurements.md"

FIXTURE_ROOT="${1:-${TMPDIR:-/tmp}/agent_slope-perf-fixture}"
SPRING_DIR="$FIXTURE_ROOT/spring-framework"
ANGULAR_DIR="$FIXTURE_ROOT/angular"

if [ ! -d "$SPRING_DIR/.git" ] || [ ! -d "$ANGULAR_DIR/.git" ]; then
  echo "error: fixture root $FIXTURE_ROOT is missing spring-framework or angular; run scripts/fetch_perf_fixture.sh first" >&2
  exit 1
fi

SPRING_SHA="$(git -C "$SPRING_DIR" rev-parse HEAD)"
ANGULAR_SHA="$(git -C "$ANGULAR_DIR" rev-parse HEAD)"

(cd "$REPO_ROOT" && cargo build --release)
BIN="$REPO_ROOT/target/release/agent_slope"

OUTPUT_DIR="$(mktemp -d)"
TIME_LOG="$(mktemp)"
trap 'rm -rf "$OUTPUT_DIR" "$TIME_LOG"' EXIT

/usr/bin/time -l "$BIN" scan "$FIXTURE_ROOT" --output "$OUTPUT_DIR" >/dev/null 2>"$TIME_LOG"

WALL_SECONDS="$(grep ' real' "$TIME_LOG" | awk '{print $1}')"
PEAK_RSS_BYTES="$(grep 'maximum resident set size' "$TIME_LOG" | awk '{print $1}')"
PEAK_RSS_MB="$(awk -v bytes="$PEAK_RSS_BYTES" 'BEGIN { printf "%.2f", bytes / 1048576 }')"

# `scanned_lines` also appears inside the per-family (java/js_ts) score
# blocks, so a plain top-level grep is only correct because `overall`
# happens to serialize first (round-2 row 3). Anchor to the `"overall": {
# ... }` block specifically (brace-depth counted, so it does not depend on
# key order inside that block either) and read `scanned_lines` from
# within it; fall back to a plain top-level grep only if that block-scoped
# read comes back empty.
OVERALL_BLOCK="$(awk '
  /"overall": \{/ { capture = 1; depth = 0 }
  capture {
    depth += gsub(/\{/, "{")
    depth -= gsub(/\}/, "}")
    print
    if (depth == 0) { exit }
  }
' "$OUTPUT_DIR/report.json")"
SCANNED_LINES="$(printf '%s\n' "$OVERALL_BLOCK" | grep -m1 '"scanned_lines"' | grep -o '[0-9]\+' || true)"
if [ -z "$SCANNED_LINES" ]; then
  SCANNED_LINES="$(grep -m1 '"scanned_lines"' "$OUTPUT_DIR/report.json" | grep -o '[0-9]\+' || true)"
fi
if [ -z "$SCANNED_LINES" ] || [ "$SCANNED_LINES" -eq 0 ]; then
  echo "error: could not read a nonzero scanned_lines count from $OUTPUT_DIR/report.json; refusing to record a row" >&2
  exit 1
fi

# `incomplete` and `skipped_files`' length (round-2 row 1): a scan that
# dropped whole files to a parse failure is not a clean, complete
# measurement, and the row must say so rather than reading as one.
# `incomplete` is a unique top-level boolean key, so a plain grep is
# sound; `relative_path` is not unique to `skipped_files` (every source
# location carries one too), so that count is taken from the
# bracket-bounded `skipped_files` array, the same way as `scanned_lines`
# above.
INCOMPLETE="$(grep -m1 '"incomplete"' "$OUTPUT_DIR/report.json" | grep -o 'true\|false')"
SKIPPED_BLOCK="$(awk '
  /"skipped_files": \[/ { capture = 1; depth = 0 }
  capture {
    depth += gsub(/\[/, "[")
    depth -= gsub(/\]/, "]")
    print
    if (depth == 0) { exit }
  }
' "$OUTPUT_DIR/report.json")"
SKIPPED_COUNT="$(printf '%s\n' "$SKIPPED_BLOCK" | grep -c '"relative_path"' || true)"

DATE="$(date -u +%Y-%m-%d)"
MACHINE="$(uname -srm)"

ROW="| $DATE | $MACHINE | spring-framework@$SPRING_SHA | angular@$ANGULAR_SHA | $SCANNED_LINES | ${WALL_SECONDS}s | ${PEAK_RSS_MB} MB | $INCOMPLETE | $SKIPPED_COUNT |"

echo "$ROW" >>"$MEASUREMENTS_FILE"
echo "$ROW"

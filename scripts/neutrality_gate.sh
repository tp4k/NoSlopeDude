#!/usr/bin/env bash
# WS-2: the operator-run half of the measurement-neutrality gate
# (nsd-plan-final.md, M0b item 8). Builds two `nsd` binaries -- the pinned
# pre-IR reference commit, in a detached worktree, and the current HEAD --
# scans the same copied fixture corpus with each, and diffs their
# `report.json` output. Because both scans target the same corpus
# directory, `scan.target` is identical in both reports by construction,
# so no normalization step is needed here (unlike `tests/neutrality.rs`,
# which compares a live scan against a baseline captured in a different
# tempdir and does normalize).
#
# This is the re-capture/comparison tool `tests/neutrality.rs`'s own doc
# comment refers to; the test suite never shells out to git or builds a
# second binary, and this script never asserts anything the committed
# baselines already cover -- it exists for a human to re-run the gate
# against history directly.
#
# Usage:
#   scripts/neutrality_gate.sh            -- compare HEAD against the pinned
#                                             pre-IR commit (default mode).
#   scripts/neutrality_gate.sh --capture  -- re-capture tests/golden/neutrality/
#                                             baselines from the current HEAD
#                                             by delegating to the Rust harness's
#                                             own NSD_NEUTRALITY_CAPTURE mode
#                                             (`cargo test --test neutrality`);
#                                             skips the pre-IR worktree/diff
#                                             logic entirely.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ "${1:-}" = "--capture" ]; then
  echo "Capturing tests/golden/neutrality/ baselines from HEAD via 'cargo test --test neutrality'..." >&2
  cd "$REPO_ROOT"
  NSD_NEUTRALITY_CAPTURE=1 cargo test --test neutrality
  exit $?
fi

# The last commit before any M0b IR-retargeting stream touched an
# analyzer (recorded at WS-2 dispatch time); this is the pre-IR reference
# the gate always compares HEAD against.
PRE_IR_SHA="5705e76522b7b0a16343cd861aa953e6ff37064a"

# The malformed corpus is exactly these three in-repo damaged fixtures
# (plan decision 1); kept in sync by hand with tests/neutrality.rs's
# MALFORMED_CORPUS_SOURCES. The clean corpus is everything else under
# tests/fixtures/ and is enumerated below, so it needs no manual sync.
MALFORMED_SOURCES=(
  "tests/fixtures/metrics/broken/Broken.ts"
  "tests/fixtures/rules/broken/Broken.java"
  "tests/fixtures/report/src/Broken.java"
)

is_malformed_source() {
  local candidate="$1"
  local malformed
  for malformed in "${MALFORMED_SOURCES[@]}"; do
    if [ "$candidate" = "$malformed" ]; then
      return 0
    fi
  done
  return 1
}

WORK_DIR="$(mktemp -d)"
WORKTREE_DIR="$WORK_DIR/pre-ir-src"

cleanup() {
  if [ -d "$WORKTREE_DIR" ]; then
    git -C "$REPO_ROOT" worktree remove --force "$WORKTREE_DIR" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# copy_corpus <sources...> <dest_root> -- mirrors each source's path
# relative to the repo root under dest_root, preserving directory
# structure, so the D16 default-exclusion globs and include_tests's
# TEST_GLOBS see the same directory shape a real scan target would.
# Fails loudly (naming both the destination and the source) on a
# collision, i.e. two different sources mapping to the same destination
# path -- this should be impossible now that paths are mirrored rather
# than flattened, but is kept as a cheap invariant check. Uses a plain
# indexed array with a linear-search helper (not `declare -A`) for
# portability with the bash 3.2 macOS ships by default.
copy_corpus() {
  local dest_root="${*: -1}"
  local sources=("${@:1:$#-1}")
  local written=()
  local source dest existing collided
  for source in "${sources[@]}"; do
    dest="$dest_root/$source"
    collided=""
    for existing in "${written[@]+"${written[@]}"}"; do
      if [ "$existing" = "$dest" ]; then
        collided="1"
        break
      fi
    done
    if [ -n "$collided" ]; then
      echo "NEUTRALITY: corpus copy collision -- $source would overwrite an earlier copy at $dest" >&2
      exit 1
    fi
    written+=("$dest")
    mkdir -p "$(dirname "$dest")"
    cp "$REPO_ROOT/$source" "$dest"
  done
}

echo "Building pre-IR reference binary from $PRE_IR_SHA in a detached worktree..." >&2
git -C "$REPO_ROOT" worktree add --detach --quiet "$WORKTREE_DIR" "$PRE_IR_SHA"
(cd "$WORKTREE_DIR" && cargo build --quiet)
PRE_IR_BIN="$WORKTREE_DIR/target/debug/nsd"

echo "Building HEAD binary..." >&2
(cd "$REPO_ROOT" && cargo build --quiet)
HEAD_BIN="$REPO_ROOT/target/debug/nsd"

# Enumerate the full corpus split: clean = all of tests/fixtures/ minus
# the three malformed sources above; malformed = exactly those three.
ALL_SOURCES=()
while IFS= read -r relative; do
  ALL_SOURCES+=("tests/fixtures/$relative")
done < <(cd "$REPO_ROOT/tests/fixtures" && find . -type f | sed 's#^\./##' | sort)

CLEAN_SOURCES=()
for source in "${ALL_SOURCES[@]+"${ALL_SOURCES[@]}"}"; do
  if ! is_malformed_source "$source"; then
    CLEAN_SOURCES+=("$source")
  fi
done

DIFF_FILTER="$WORK_DIR/first_diff.jq"
cat > "$DIFF_FILTER" << 'JQ_EOF'
def all_diffs(a; b; path):
  if a == b then empty
  elif (a|type) != (b|type) then path
  elif (a|type) == "object" then
    (((a|keys) + (b|keys)) | unique)[] as $k
    | all_diffs(a[$k]; b[$k]; path + "/" + $k)
  elif (a|type) == "array" then
    if (a|length) != (b|length) then path
    else
      range(0; a|length) as $i
      | all_diffs(a[$i]; b[$i]; path + "/" + ($i|tostring))
    end
  else
    path
  end;

[all_diffs($a[0]; $b[0]; "")]
JQ_EOF

identical_count=0
for label in clean malformed; do
  if [ "$label" = "clean" ]; then
    sources=("${CLEAN_SOURCES[@]+"${CLEAN_SOURCES[@]}"}")
  else
    sources=("${MALFORMED_SOURCES[@]}")
  fi

  corpus_dir="$WORK_DIR/corpus-$label"
  copy_corpus "${sources[@]}" "$corpus_dir"

  pre_ir_out="$WORK_DIR/out-$label-pre-ir"
  head_out="$WORK_DIR/out-$label-head"
  mkdir -p "$pre_ir_out" "$head_out"
  "$PRE_IR_BIN" scan "$corpus_dir" --output "$pre_ir_out" --include-tests >/dev/null
  "$HEAD_BIN" scan "$corpus_dir" --output "$head_out" --include-tests >/dev/null

  diffs="$(jq -c -n --slurpfile a "$pre_ir_out/report.json" --slurpfile b "$head_out/report.json" -f "$DIFF_FILTER")"
  first_diff="$(echo "$diffs" | jq -r '.[0] // empty')"
  if [ -n "$first_diff" ]; then
    echo "NEUTRALITY: $label corpus diverged at $first_diff" >&2
    exit 1
  fi
  identical_count=$((identical_count + 1))
done

echo "NEUTRALITY: identical on $identical_count corpora"

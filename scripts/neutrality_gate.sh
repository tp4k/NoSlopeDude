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
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# The last commit before any M0b IR-retargeting stream touched an
# analyzer (recorded at WS-2 dispatch time); this is the pre-IR reference
# the gate always compares HEAD against.
PRE_IR_SHA="5705e76522b7b0a16343cd861aa953e6ff37064a"

# Kept in sync by hand with tests/neutrality.rs's CLEAN_CORPUS_SOURCES /
# MALFORMED_CORPUS_SOURCES -- one "<label> <space-separated source list>"
# pair per corpus.
CORPUS_LABELS=(clean malformed)
CORPUS_clean="tests/fixtures/rules/__tests__/CleanJava.java tests/fixtures/rules/__tests__/CleanJs.js tests/fixtures/rules/__tests__/CleanTs.ts"
CORPUS_malformed="tests/fixtures/rules/broken/Broken.java tests/fixtures/rules/broken/Good.java"

WORK_DIR="$(mktemp -d)"
WORKTREE_DIR="$WORK_DIR/pre-ir-src"

cleanup() {
  if [ -d "$WORKTREE_DIR" ]; then
    git -C "$REPO_ROOT" worktree remove --force "$WORKTREE_DIR" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# copy_corpus <sources...> <dest_root> -- flattens each source file into
# dest_root/src/<file name>, the same flattening tests/neutrality.rs's
# copy_corpus does, so a source directory name such as __tests__ or
# broken never lands somewhere the D16 default exclusions would skip.
copy_corpus() {
  local dest_root="${*: -1}"
  local sources=("${@:1:$#-1}")
  mkdir -p "$dest_root/src"
  local source
  for source in "${sources[@]}"; do
    cp "$REPO_ROOT/$source" "$dest_root/src/$(basename "$source")"
  done
}

echo "Building pre-IR reference binary from $PRE_IR_SHA in a detached worktree..." >&2
git -C "$REPO_ROOT" worktree add --detach --quiet "$WORKTREE_DIR" "$PRE_IR_SHA"
(cd "$WORKTREE_DIR" && cargo build --quiet)
PRE_IR_BIN="$WORKTREE_DIR/target/debug/nsd"

echo "Building HEAD binary..." >&2
(cd "$REPO_ROOT" && cargo build --quiet)
HEAD_BIN="$REPO_ROOT/target/debug/nsd"

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
for label in "${CORPUS_LABELS[@]}"; do
  sources_var="CORPUS_$label"
  # shellcheck disable=SC2086
  # Intentional: ${!sources_var} holds a space-separated list of relative
  # paths, not a single value, and must word-split into copy_corpus's args.
  read -r -a sources <<< "${!sources_var}"

  corpus_dir="$WORK_DIR/corpus-$label"
  copy_corpus "${sources[@]}" "$corpus_dir"

  pre_ir_out="$WORK_DIR/out-$label-pre-ir"
  head_out="$WORK_DIR/out-$label-head"
  mkdir -p "$pre_ir_out" "$head_out"
  "$PRE_IR_BIN" scan "$corpus_dir" --output "$pre_ir_out" >/dev/null
  "$HEAD_BIN" scan "$corpus_dir" --output "$head_out" >/dev/null

  diffs="$(jq -c -n --slurpfile a "$pre_ir_out/report.json" --slurpfile b "$head_out/report.json" -f "$DIFF_FILTER")"
  first_diff="$(echo "$diffs" | jq -r '.[0] // empty')"
  if [ -n "$first_diff" ]; then
    echo "NEUTRALITY: $label corpus diverged at $first_diff" >&2
    exit 1
  fi
  identical_count=$((identical_count + 1))
done

echo "NEUTRALITY: identical on $identical_count corpora"

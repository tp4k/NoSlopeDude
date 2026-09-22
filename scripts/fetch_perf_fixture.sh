#!/usr/bin/env bash
# WS-6/D25: shallow-fetches the two pinned public repositories the perf
# measurement scans (spring-framework for the Java half, angular for the
# JS/TS half) into one fixture root OUTSIDE this repo's own tree (D25: an
# in-repo, even gitignored, root would be invisible to the scanner's own
# walk). Idempotent: a fixture root that already holds both repos is
# reused rather than refetched. Requires network; with none, this exits
# nonzero naming both URLs, per the observable acceptance.
set -euo pipefail

FIXTURE_ROOT="${1:-${TMPDIR:-/tmp}/nsd-perf-fixture}"

SPRING_URL="https://github.com/spring-projects/spring-framework"
SPRING_SHA="e8eb2b6751ca6efa2a6b8a8eb930ed3469ebafb9"
SPRING_DIR="$FIXTURE_ROOT/spring-framework"

ANGULAR_URL="https://github.com/angular/angular"
ANGULAR_SHA="a783c4e7b753929ababa610e305112b82aaa0eb0"
ANGULAR_DIR="$FIXTURE_ROOT/angular"

# fetch_pinned <url> <sha> <dir> — fetches the pinned sha at depth 1 (D25);
# falls back to a shallow clone of the default branch head if the pin is
# unreachable, printing a warning and the sha actually obtained. Reuses
# `dir` as-is only if it holds a valid completed clone (a `.git` with a
# resolvable HEAD commit); a `.git` left behind by an interrupted fetch is
# repaired (deleted and refetched) rather than silently reused. If `dir`
# exists without a `.git` at all, this script did not create it, so it is
# left alone and the fetch is refused (round-2 row 4).
fetch_pinned() {
  local url="$1" sha="$2" dir="$3"
  if [ -d "$dir/.git" ]; then
    if git -C "$dir" rev-parse --verify --quiet HEAD^{commit} >/dev/null; then
      echo "reusing existing fixture: $dir ($(git -C "$dir" rev-parse HEAD))"
      return 0
    fi
    echo "warning: $dir has a .git with no resolvable commit (an interrupted fetch); repairing it" >&2
    rm -rf "$dir"
  elif [ -e "$dir" ]; then
    echo "error: $dir already exists and is not a git checkout this script created; refusing to delete it" >&2
    return 1
  fi
  mkdir -p "$dir"
  if (
    cd "$dir" &&
    git init --quiet &&
    git remote add origin "$url" &&
    git fetch --quiet --depth 1 origin "$sha" &&
    git checkout --quiet --detach FETCH_HEAD
  ); then
    echo "fetched $url at pinned sha $(git -C "$dir" rev-parse HEAD)"
    return 0
  fi
  echo "warning: pinned sha $sha unreachable for $url; falling back to the default branch head" >&2
  if [ -d "$dir/.git" ]; then
    rm -rf "$dir"
  elif [ -e "$dir" ]; then
    echo "error: $dir already exists and is not a git checkout this script created; refusing to delete it" >&2
    return 1
  fi
  if git clone --quiet --depth 1 --single-branch "$url" "$dir"; then
    echo "fetched $url at fallback sha $(git -C "$dir" rev-parse HEAD)"
    return 0
  fi
  return 1
}

if ! fetch_pinned "$SPRING_URL" "$SPRING_SHA" "$SPRING_DIR" ||
  ! fetch_pinned "$ANGULAR_URL" "$ANGULAR_SHA" "$ANGULAR_DIR"; then
  echo "error: could not fetch the perf fixture; check network access to $SPRING_URL and $ANGULAR_URL" >&2
  exit 1
fi

echo "perf fixture ready at $FIXTURE_ROOT"

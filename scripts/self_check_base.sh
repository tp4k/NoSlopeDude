#!/usr/bin/env bash
# Prints the `nsd check --base` argument for .github/workflows/self-check.yml.
#   self_check_base.sh <event> <before> <base_ref>
# pull_request -> origin/<base_ref>; push -> <before>, or HEAD^ when <before>
# is not a commit here (the all-zero SHA of a new branch never is); HEAD without a parent prints nothing.
set -euo pipefail

main() {
  if [[ $# -ne 3 ]]; then
    echo "error: usage: self_check_base.sh <event> <before> <base_ref>" >&2
    return 2
  fi
  local event="$1" before="$2" base_ref="$3"
  case "$event" in
    pull_request)
      if [[ -z "$base_ref" ]]; then
        echo "error: pull_request event without a base ref" >&2
        return 1
      fi
      echo "origin/$base_ref"
      ;;
    push)
      if git cat-file -e "${before}^{commit}" 2>/dev/null; then
        echo "$before"
      elif git rev-parse --verify --quiet "HEAD^" >/dev/null; then
        echo "HEAD^"
      else
        echo "notice: HEAD has no parent commit; skipping nsd check" >&2
      fi
      ;;
    *)
      echo "error: unsupported event '$event'" >&2
      return 1
      ;;
  esac
}

main "$@"

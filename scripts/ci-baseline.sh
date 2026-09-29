#!/usr/bin/env bash
# Prints the commit of the newest successful push-triggered run of WORKFLOW on
# BRANCH when that commit is an ancestor of HEAD, and nothing otherwise.
#
# That run's gate passed, so every suite held at its commit, and testing the
# suites affected since then covers HEAD. The previous push is not a safe base:
# its run may have failed, or been superseded while pending and never run, and
# diffing from it would skip the suites that change broke. An empty result
# makes CI select every suite.
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 <workflow file> <branch> <head sha>" >&2
  exit 2
fi
workflow="$1"
branch="$2"
head_sha="$3"

if ! base="$(
  gh run list --workflow "${workflow}" --branch "${branch}" --event push \
    --status success --limit 1 --json headSha --jq '.[0].headSha // empty'
)"; then
  echo "::warning::could not list successful ${workflow} runs; selecting every CI suite" >&2
  exit 0
fi

[[ "${base}" =~ ^[0-9a-f]{40}$ ]] || exit 0
git cat-file -e "${base}^{commit}" 2>/dev/null || exit 0
git merge-base --is-ancestor "${base}" "${head_sha}" || exit 0
printf '%s\n' "${base}"

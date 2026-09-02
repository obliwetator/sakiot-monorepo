#!/usr/bin/env bash
set -euo pipefail

token_file="${SAKIOT_GITHUB_TOKEN_FILE:-}"
if [[ -z "${token_file}" || ! -f "${token_file}" || -L "${token_file}" ]]; then
  echo "GitHub token file unavailable" >&2
  exit 1
fi

case "${1:-}" in
  *Username* | *username*)
    printf '%s\n' "x-access-token"
    ;;
  *Password* | *password*)
    cat -- "${token_file}"
    ;;
  *)
    echo "unsupported Git credential prompt" >&2
    exit 1
    ;;
esac

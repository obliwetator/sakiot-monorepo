#!/usr/bin/env bash
set -euo pipefail

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
lock="${temporary}/deploy.lock"

bash -c 'exec 9>"$1"; flock 9; sleep 2' _ "${lock}" &
holder=$!
sleep 0.2

if flock -n "${lock}" true; then
  echo "deployment lock allowed overlap" >&2
  kill "${holder}" 2>/dev/null || true
  wait "${holder}" 2>/dev/null || true
  exit 1
fi
wait "${holder}"

flock -n "${lock}" true

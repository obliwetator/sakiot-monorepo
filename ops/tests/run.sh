#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for test_script in "${test_dir}"/*_test.sh; do
  echo "==> $(basename "${test_script}")"
  "${test_script}"
done

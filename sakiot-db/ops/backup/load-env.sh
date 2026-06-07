#!/usr/bin/env bash

MONOREPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SAKIOT_ENV_FILE="${SAKIOT_ENV_FILE:-${MONOREPO_ROOT}/.env}"

if [[ ! -f "${SAKIOT_ENV_FILE}" ]]; then
  echo "missing environment file: ${SAKIOT_ENV_FILE}" >&2
  return 1
fi

set -a
# shellcheck disable=SC1090
source "${SAKIOT_ENV_FILE}"
set +a

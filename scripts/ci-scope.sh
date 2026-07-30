#!/usr/bin/env bash
set -euo pipefail

rust=false
api_contract=false
frontend=false
ops=false
unknown=false

while IFS= read -r path; do
  [[ -n "${path}" ]] || continue
  case "${path}" in
    *.md|LICENSE*|docs/*)
      ;;
    Cargo.toml|Cargo.lock)
      rust=true
      api_contract=true
      ;;
    rust-toolchain.toml|clippy.toml|.cargo/*|.sqlx/*|\
    fbi-agent/*|sakiot-paths/*|sakiot-storage/*|sakiot-db/migrations/*|\
    scripts/sqlx-*)
      rust=true
      ;;
    sakiot-proto/*)
      rust=true
      api_contract=true
      ;;
    web-server/*)
      rust=true
      api_contract=true
      ;;
    ops/sakiot-deploy/*)
      rust=true
      ops=true
      ;;
    ops/*|sakiot-db/ops/*)
      ops=true
      ;;
    sakiot-stage/scripts/generate-api-types.ts|\
    sakiot-stage/src/api/openapi.ts|\
    sakiot-stage/package.json|sakiot-stage/bun.lock)
      frontend=true
      api_contract=true
      ;;
    sakiot-stage/*)
      frontend=true
      ;;
    *)
      unknown=true
      ;;
  esac
done

if [[ "${unknown}" == true ]]; then
  rust=true
  api_contract=true
  frontend=true
  ops=true
fi

printf 'rust=%s\n' "${rust}"
printf 'api_contract=%s\n' "${api_contract}"
printf 'frontend=%s\n' "${frontend}"
printf 'ops=%s\n' "${ops}"

#!/usr/bin/env bash
set -euo pipefail

rust=false
dsp=false
api_contract=false
frontend=false
ops=false
unknown=false

while IFS= read -r path; do
  [[ -n "${path}" ]] || continue
  case "${path}" in
    *.md|LICENSE*|docs/*)
      ;;
    # The root manifest also carries the workspace lints, which the DSP job's
    # wasm-feature Clippy pass applies.
    Cargo.toml|Cargo.lock)
      rust=true
      dsp=true
      api_contract=true
      ;;
    # The toolchain and Cargo config also build the WASM that the DSP and
    # frontend suites compile.
    rust-toolchain.toml|.cargo/*)
      rust=true
      dsp=true
      frontend=true
      ;;
    clippy.toml)
      rust=true
      dsp=true
      ;;
    .sqlx/*|fbi-agent/*|sakiot-paths/*|sakiot-storage/*|\
    sakiot-db/migrations/*|scripts/sqlx-*)
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
    sakiot-dsp/*)
      rust=true
      dsp=true
      frontend=true
      ;;
    ops/sakiot-dev/*|ops/sakiot-deploy/*)
      rust=true
      ops=true
      ;;
    ops/*|sakiot-db/ops/*)
      ops=true
      ;;
    sakiot-stage/scripts/generate-api-types.ts|\
    sakiot-stage/scripts/codegen/*|\
    sakiot-stage/src/api/openapi.ts|\
    sakiot-stage/package.json|sakiot-stage/bun.lock)
      frontend=true
      api_contract=true
      ;;
    sakiot-stage/*)
      frontend=true
      ;;
    # The ops suite includes the workflow injection guard, which must run
    # whenever a workflow changes, and the Git hook tests.
    .github/workflows/*|.githooks/*)
      ops=true
      ;;
    .github/*)
      ;;
    *)
      unknown=true
      ;;
  esac
done

if [[ "${unknown}" == true ]]; then
  rust=true
  dsp=true
  api_contract=true
  frontend=true
  ops=true
fi

printf 'rust=%s\n' "${rust}"
printf 'dsp=%s\n' "${dsp}"
printf 'api_contract=%s\n' "${api_contract}"
printf 'frontend=%s\n' "${frontend}"
printf 'ops=%s\n' "${ops}"

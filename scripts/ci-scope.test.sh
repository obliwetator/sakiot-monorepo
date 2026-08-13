#!/usr/bin/env bash
set -euo pipefail

assert_scope() {
  local name="$1"
  local rust="$2"
  local dsp="$3"
  local api_contract="$4"
  local frontend="$5"
  local ops="$6"
  shift 6

  local actual
  local expected
  actual="$(printf '%s\n' "$@" | scripts/ci-scope.sh)"
  expected="$(
    printf 'rust=%s\ndsp=%s\napi_contract=%s\nfrontend=%s\nops=%s' \
      "${rust}" "${dsp}" "${api_contract}" "${frontend}" "${ops}"
  )"

  if [[ "${actual}" != "${expected}" ]]; then
    printf 'scope case failed: %s\nexpected:\n%s\nactual:\n%s\n' \
      "${name}" "${expected}" "${actual}" >&2
    return 1
  fi
}

assert_scope \
  "frontend only" \
  false false false true false \
  "sakiot-stage/src/features/audio-dashboard/AudioEventTimeline.tsx"

assert_scope \
  "rust service only" \
  true false false false false \
  "fbi-agent/src/main.rs"

assert_scope \
  "web API contract" \
  true false true false false \
  "web-server/src/audio/sessions.rs"

assert_scope \
  "generated frontend API contract" \
  false false true true false \
  "sakiot-stage/src/api/openapi.ts"

assert_scope \
  "protobuf contract" \
  true false true false false \
  "sakiot-proto/proto/voice.proto"

assert_scope \
  "deploy engine" \
  true false false false true \
  "ops/sakiot-deploy/src/deploy.rs"

assert_scope \
  "development CLI" \
  true false false false true \
  "ops/sakiot-dev/src/cli.rs"

assert_scope \
  "mixed frontend and rust" \
  true false false true false \
  "sakiot-stage/src/index.tsx" \
  "sakiot-storage/src/lib.rs"

assert_scope \
  "docs only" \
  false false false false false \
  "README.md" \
  "docs/operations.md"

assert_scope \
  "unknown path fails closed" \
  true true true true true \
  "compose.dev.yml"

assert_scope \
  "CI config only" \
  false false false false false \
  ".github/workflows/ci.yml" \
  ".github/workflows/cache-cleanup.yml"

assert_scope \
  "shared DSP" \
  true true false true false \
  "sakiot-DSP/src/lib.rs" \
  "sakiot-DSP/pkg/sakiot_dsp_bg.wasm"

echo "ci-scope tests passed"

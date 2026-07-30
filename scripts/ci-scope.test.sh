#!/usr/bin/env bash
set -euo pipefail

assert_scope() {
  local name="$1"
  local rust="$2"
  local api_contract="$3"
  local frontend="$4"
  local ops="$5"
  shift 5

  local actual
  local expected
  actual="$(printf '%s\n' "$@" | scripts/ci-scope.sh)"
  expected="$(
    printf 'rust=%s\napi_contract=%s\nfrontend=%s\nops=%s' \
      "${rust}" "${api_contract}" "${frontend}" "${ops}"
  )"

  if [[ "${actual}" != "${expected}" ]]; then
    printf 'scope case failed: %s\nexpected:\n%s\nactual:\n%s\n' \
      "${name}" "${expected}" "${actual}" >&2
    return 1
  fi
}

assert_scope \
  "frontend only" \
  false false true false \
  "sakiot-stage/src/features/audio-dashboard/AudioEventTimeline.tsx"

assert_scope \
  "rust service only" \
  true false false false \
  "fbi-agent/src/main.rs"

assert_scope \
  "web API contract" \
  true true false false \
  "web-server/src/audio/sessions.rs"

assert_scope \
  "generated frontend API contract" \
  false true true false \
  "sakiot-stage/src/api/openapi.ts"

assert_scope \
  "protobuf contract" \
  true true false false \
  "sakiot-proto/proto/voice.proto"

assert_scope \
  "deploy engine" \
  true false false true \
  "ops/sakiot-deploy/src/deploy.rs"

assert_scope \
  "mixed frontend and rust" \
  true false true false \
  "sakiot-stage/src/index.tsx" \
  "sakiot-storage/src/lib.rs"

assert_scope \
  "docs only" \
  false false false false \
  "README.md" \
  "docs/operations.md"

assert_scope \
  "unknown path fails closed" \
  true true true true \
  ".github/workflows/ci.yml"

echo "ci-scope tests passed"

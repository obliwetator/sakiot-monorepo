#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/common.sh
source "${test_dir}/../lib/common.sh"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  [[ "$1" == "$2" ]] || fail "$3 (expected '$2', got '$1')"
}

SHA_A="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
SHA_B="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

root="$(mktemp -d)"
trap 'rm -rf "${root}"' EXIT
releases="${root}/releases"
mkdir -p "${releases}"

# Helper to fabricate a release dir with a manifest and chosen artifacts.
make_release() {
  local id="$1" sha="$2"; shift 2
  local dir="${releases}/${id}"
  mkdir -p "${dir}"
  printf '{"sha":"%s"}\n' "${sha}" >"${dir}/manifest.json"
  local comp
  for comp in "$@"; do
    case "${comp}" in
      bot)      mkdir -p "${dir}/fbi-agent"; printf 'bin\n' >"${dir}/fbi-agent/fbi_agent" ;;
      web)      mkdir -p "${dir}/web"; printf 'bin\n' >"${dir}/web/web_server" ;;
      frontend) mkdir -p "${dir}/frontend/dist"; printf 'x\n' >"${dir}/frontend/dist/index.html" ;;
      bot-empty) mkdir -p "${dir}/fbi-agent"; : >"${dir}/fbi-agent/fbi_agent" ;;
      frontend-empty) mkdir -p "${dir}/frontend/dist" ;;
    esac
  done
  printf '%s\n' "${dir}"
}

# Oldest -> newest by timestamp in the id.
old_a="$(make_release "v1.0.1-aaaaaaaaaaaa-20260101T120000Z" "${SHA_A}" bot)"
new_a="$(make_release "v1.0.1-aaaaaaaaaaaa-20260102T120000Z" "${SHA_A}" bot web frontend)"
other_b="$(make_release "v1.0.2-bbbbbbbbbbbb-20260103T120000Z" "${SHA_B}" bot web frontend)"

# 1. newest matching dir for the SHA is chosen for each component
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" bot "")"      "${new_a}" "bot picks newest A"
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" web "")"      "${new_a}" "web picks newest A"
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" frontend "")" "${new_a}" "frontend picks newest A"

# 2. excluding the newest falls back to the older dir that still has the component
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" bot "${new_a}")" "${old_a}" "bot falls back past excluded"
# ...but older A has no web/frontend -> empty (would rebuild)
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" web "${new_a}")" "" "no older web for A"

# 3. unknown SHA -> empty
assert_eq "$(reusable_artifact "${releases}" "deadbeef" bot "")" "" "unknown sha -> empty"

# 4. other SHA's artifacts are not used for SHA_A
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" bot "${new_a}")" "${old_a}" "does not leak B into A"

# 5. empty artifact files / dirs are not matched
empty_a="$(make_release "v1.0.3-cccccccccccc-20260104T120000Z" "${SHA_A}" bot-empty frontend-empty)"
# newest A (new_a) still has real artifacts, so it should still win over empty_a
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" bot "")"      "${new_a}" "empty bot bin ignored, real wins"
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" frontend "")" "${new_a}" "empty dist ignored, real wins"
# excluding the real one, the empty one must NOT match
assert_eq "$(reusable_artifact "${releases}" "${SHA_A}" frontend "${new_a}")" "" "empty dist not matched"

echo "reuse_artifact_test ok"

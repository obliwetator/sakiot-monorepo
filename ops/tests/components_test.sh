#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/components.sh
source "${test_dir}/../lib/components.sh"

assert_components() {
  local expected="$1"
  shift
  local actual
  actual="$(components_for_paths "$@")"
  [[ "${actual}" == "${expected}" ]] || {
    printf 'expected:\n%s\nactual:\n%s\n' "${expected}" "${actual}" >&2
    exit 1
  }
}

assert_components "bot" "FBI-agent/src/main.rs"
assert_components "web" "web_server/src/main.rs"
assert_components "frontend" "sakiot_stage/src/main.tsx"
assert_components $'bot\nweb' "sakiot-proto/proto/fbi_agent.proto"
assert_components $'database\nbot\nweb' "sakiot-db/migrations/20260101000000_test.sql"
assert_components $'database\nbot\nweb\nfrontend' "unknown-root-file"
assert_components "" "README.md" "docs/runbook.md"
# ops/ is installed out-of-band; tag-time ops/ changes are a no-op
assert_components "" "ops/use-legacy-data.sh"
assert_components "" "ops/lib/components.sh" "ops/systemd/sakiot-web.service"
# but an ops/ change alongside a real source change must not mask it
assert_components "bot" "ops/use-legacy-data.sh" "FBI-agent/src/main.rs"
# sakiot-db/ops (backup scripts) still fan out, distinct from ops/
assert_components $'bot\nweb\nfrontend' "sakiot-db/ops/backup/pre-migrate-backup.sh"
assert_components $'database\nbot\nweb\nfrontend'

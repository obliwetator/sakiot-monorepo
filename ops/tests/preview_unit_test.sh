#!/usr/bin/env bash
# A preview slot's unit is the staging unit with the slot's own paths and unit
# names, the shared preview env file, and the shared sakiot-preview user.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
render="${test_dir}/../render-preview-unit.sh"

fail() {
  echo "preview unit: $*" >&2
  exit 1
}

unit="$("${render}" clip-editor /etc/sakiot/preview.env)"
for expected in \
  "User=sakiot-preview" \
  "Group=sakiot-preview" \
  "UMask=0007" \
  "WorkingDirectory=/var/lib/sakiot-preview-clip-editor/data" \
  "EnvironmentFile=/etc/sakiot/preview.env" \
  "EnvironmentFile=/srv/sakiot-preview-clip-editor/current/web/service.env" \
  "ExecStart=/srv/sakiot-preview-clip-editor/current/web/web_server" \
  "ProtectSystem=strict" \
  "ReadWritePaths=/var/lib/sakiot-preview-clip-editor/data"; do
  grep -qxF -- "${expected}" <<<"${unit}" || fail "missing '${expected}'"
done
if grep -q "sakiot-staging\|staging.env" <<<"${unit}"; then
  fail "staging names or env file leaked into a preview unit"
fi

for arguments in \
  "Bad_Slot /etc/sakiot/preview.env" \
  "clip-editor relative.env" \
  "clip-editor /etc/sakiot/preview.env|x"; do
  read -r -a argv <<<"${arguments}"
  if "${render}" "${argv[@]}" >/dev/null 2>&1; then
    fail "accepted invalid arguments: ${arguments}"
  fi
done

echo "preview unit: ok"

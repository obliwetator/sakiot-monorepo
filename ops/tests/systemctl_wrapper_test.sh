#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="${test_dir}/../systemctl-wrapper"

for arguments in \
  "restart ssh.service" \
  "start postgresql.service" \
  "stop postgresql.service" \
  "start sakiot-fbi-agent@good.service extra.service" \
  "is-active sakiot-fbi-agent@good.service" \
  "legacy-bot-is-active ssh.service" \
  "legacy-bot-disable fbi-agent@good.service extra.service" \
  "legacy-bot-enable ssh.service" \
  "legacy-web-stop-disable extra" \
  "daemon-reload"; do
  read -r -a argv <<<"${arguments}"
  if "${wrapper}" "${argv[@]}" >/dev/null 2>&1; then
    echo "systemctl wrapper accepted invalid command: ${arguments}" >&2
    exit 1
  fi
done

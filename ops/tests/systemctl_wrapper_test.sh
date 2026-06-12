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
  "legacy-bot-restart ssh.service" \
  "legacy-web-stop-disable extra" \
  "enable-web extra" \
  "enable-web sakiot-staging-web.service.evil" \
  "enable-web sakiot-web.service extra" \
  "restart sakiot-web.service extra.service" \
  "restart evil-sakiot-staging-web.service" \
  "stop sakiot-staging-fbi-agent@.service" \
  "is-active --quiet sakiot-staging-web.service" \
  "start sakiot-staging-fbi-agent@good.service extra.service" \
  "kill-bot postgresql.service" \
  "kill-bot sakiot-web.service" \
  "kill-bot sakiot-fbi-agent@.service" \
  "kill-bot sakiot-fbi-agent@good.service extra.service" \
  "kill-bot" \
  "daemon-reload"; do
  read -r -a argv <<<"${arguments}"
  if "${wrapper}" "${argv[@]}" >/dev/null 2>&1; then
    echo "systemctl wrapper accepted invalid command: ${arguments}" >&2
    exit 1
  fi
done

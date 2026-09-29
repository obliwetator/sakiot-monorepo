#!/usr/bin/env bash
# Prints the systemd unit of one preview slot's web server, derived from the
# staging unit: slot-specific names and paths, the shared preview env file, and
# the shared sakiot-preview runtime user.
#
#   ops/render-preview-unit.sh <slot> <env file>
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 <slot> <env file>" >&2
  exit 2
fi
slot="$1"
env_file="$2"
[[ "${slot}" =~ ^[a-z0-9][a-z0-9-]{0,31}$ ]] || { echo "invalid slot '${slot}'" >&2; exit 2; }
[[ "${env_file}" =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo "invalid env file '${env_file}'" >&2; exit 2; }

ops_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The user lines are rewritten first: every slot shares one runtime user,
# while paths and unit names are per slot.
sed \
  -e 's|^User=sakiot-staging$|User=sakiot-preview|' \
  -e 's|^Group=sakiot-staging$|Group=sakiot-preview|' \
  -e "s|/etc/sakiot/staging.env|${env_file}|g" \
  -e "s|sakiot-staging|sakiot-preview-${slot}|g" \
  "${ops_dir}/systemd/sakiot-staging-web.service"

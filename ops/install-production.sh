#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi
if [[ "$#" -ne 1 || ! -f "$1" ]]; then
  echo "usage: ops/install-production.sh /path/to/deploy-key.pub" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
install_root="/usr/local/lib/sakiot-deploy"
public_key="$(cat "$1")"

if ! id sakiot >/dev/null 2>&1; then
  useradd --create-home --home-dir /var/lib/sakiot --shell /bin/bash sakiot
fi
passwd -l sakiot >/dev/null

install -d -o root -g sakiot -m 0750 /etc/sakiot
install -d -o sakiot -g sakiot -m 0750 \
  /var/lib/sakiot/data \
  /var/lib/sakiot/deploy \
  /var/cache/sakiot \
  /srv/sakiot/releases \
  /srv/sakiot/current
install -d -o sakiot -g sakiot -m 0755 /var/www/patrykstyla.com

rm -rf "${install_root}"
install -d -o root -g root -m 0755 "${install_root}"
cp -a "${script_dir}/." "${install_root}/"
chown -R root:root "${install_root}"
find "${install_root}" -type d -exec chmod 0755 {} +
find "${install_root}" -type f -name '*.sh' -exec chmod 0755 {} +
chmod 0755 "${install_root}/deploy" "${install_root}/ssh/forced-command"
chmod 0755 "${install_root}/systemctl-wrapper"

install -m 0644 "${script_dir}/systemd/sakiot-fbi-agent@.service" \
  /etc/systemd/system/sakiot-fbi-agent@.service
install -m 0644 "${script_dir}/systemd/sakiot-web.service" \
  /etc/systemd/system/sakiot-web.service
visudo -cf "${script_dir}/sudoers/sakiot-deploy"
install -m 0440 "${script_dir}/sudoers/sakiot-deploy" \
  /etc/sudoers.d/sakiot-deploy

if [[ ! -e /etc/sakiot/production.env ]]; then
  install -o root -g sakiot -m 0640 \
    "${script_dir}/production.env.example" /etc/sakiot/production.env
fi

install -d -o sakiot -g sakiot -m 0700 /var/lib/sakiot/.ssh
authorized_keys="/var/lib/sakiot/.ssh/authorized_keys"
forced_options="restrict,command=\"${install_root}/ssh/forced-command\""
printf '%s %s\n' "${forced_options}" "${public_key}" >"${authorized_keys}"
chown sakiot:sakiot "${authorized_keys}"
chmod 0600 "${authorized_keys}"

systemctl daemon-reload
echo "production skeleton installed; edit /etc/sakiot/production.env before first tag"

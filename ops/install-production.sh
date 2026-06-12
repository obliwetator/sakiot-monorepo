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
  /var/lib/sakiot/backups \
  /var/cache/sakiot \
  /srv/sakiot/releases \
  /srv/sakiot/current
install -d -o sakiot -g sakiot -m 0755 /var/www/patrykstyla.com

# Staging instance: same user, separate layout.
install -d -o sakiot -g sakiot -m 0750 \
  /var/lib/sakiot-staging/data \
  /var/lib/sakiot-staging/deploy \
  /var/lib/sakiot-staging/backups \
  /var/cache/sakiot-staging \
  /srv/sakiot-staging/releases \
  /srv/sakiot-staging/current
install -d -o sakiot -g sakiot -m 0755 /var/www/staging.patrykstyla.com

rm -rf "${install_root}"
install -d -o root -g root -m 0755 "${install_root}"
cp -a "${script_dir}/." "${install_root}/"
# The Rust engine is installed as a built binary below, never as source.
rm -rf "${install_root}/sakiot-deploy"
install -d -o root -g root -m 0755 "${install_root}/backup"
cp -a "${script_dir}/../sakiot-db/ops/backup/." "${install_root}/backup/"
cp -a "${script_dir}/../sakiot-db/migrations" "${install_root}/backup/migrations"
chown -R root:root "${install_root}"
find "${install_root}" -type d -exec chmod 0755 {} +
find "${install_root}" -type f -name '*.sh' -exec chmod 0755 {} +
chmod 0755 "${install_root}/deploy" "${install_root}/ssh/forced-command"
chmod 0755 "${install_root}/systemctl-wrapper"

# Rust deploy engine (ops/sakiot-deploy). Built from this checkout as the
# sakiot user (cargo lives in its home), installed root-owned alongside the
# bash engine. SAKIOT_DEPLOY_ENGINE in the env files selects which one runs.
if sudo -u sakiot bash -lc 'command -v cargo' >/dev/null 2>&1; then
  repo_root="$(cd "${script_dir}/.." && pwd)"
  sudo -u sakiot bash -lc \
    "cd '${repo_root}' && CARGO_TARGET_DIR=/var/cache/sakiot/cargo-target \
     cargo build --release --locked --package sakiot-deploy"
  install -d -o root -g root -m 0755 "${install_root}/bin"
  install -o root -g root -m 0755 \
    /var/cache/sakiot/cargo-target/release/sakiot-deploy \
    "${install_root}/bin/sakiot-deploy"
else
  echo "cargo unavailable for sakiot; Rust deploy engine not installed" >&2
  echo "(SAKIOT_DEPLOY_ENGINE=rust will refuse to run until ops/update-deploy-engine.sh succeeds)" >&2
fi

install -m 0644 "${script_dir}/systemd/sakiot-fbi-agent@.service" \
  /etc/systemd/system/sakiot-fbi-agent@.service
install -m 0644 "${script_dir}/systemd/sakiot-web.service" \
  /etc/systemd/system/sakiot-web.service
install -m 0644 "${script_dir}/systemd/sakiot-staging-fbi-agent@.service" \
  /etc/systemd/system/sakiot-staging-fbi-agent@.service
install -m 0644 "${script_dir}/systemd/sakiot-staging-web.service" \
  /etc/systemd/system/sakiot-staging-web.service
install -m 0644 "${script_dir}/systemd/sakiot-db-backup@.service" \
  /etc/systemd/system/sakiot-db-backup@.service
install -m 0644 "${script_dir}/systemd/sakiot-db-restore-test.service" \
  /etc/systemd/system/sakiot-db-restore-test.service
install -m 0644 "${script_dir}/systemd/sakiot-db-backup-hourly.timer" \
  /etc/systemd/system/sakiot-db-backup-hourly.timer
install -m 0644 "${script_dir}/systemd/sakiot-db-backup-nightly.timer" \
  /etc/systemd/system/sakiot-db-backup-nightly.timer
install -m 0644 "${script_dir}/systemd/sakiot-db-restore-test.timer" \
  /etc/systemd/system/sakiot-db-restore-test.timer
visudo -cf "${script_dir}/sudoers/sakiot-deploy"
install -m 0440 "${script_dir}/sudoers/sakiot-deploy" \
  /etc/sudoers.d/sakiot-deploy

if [[ ! -e /etc/sakiot/production.env ]]; then
  install -o root -g sakiot -m 0640 \
    "${script_dir}/production.env.example" /etc/sakiot/production.env
fi
if [[ ! -e /etc/sakiot/staging.env ]]; then
  install -o root -g sakiot -m 0640 \
    "${script_dir}/staging.env.example" /etc/sakiot/staging.env
fi

install -d -o sakiot -g sakiot -m 0700 /var/lib/sakiot/.ssh
authorized_keys="/var/lib/sakiot/.ssh/authorized_keys"
forced_options="restrict,command=\"${install_root}/ssh/forced-command\""
printf '%s %s\n' "${forced_options}" "${public_key}" >"${authorized_keys}"
chown sakiot:sakiot "${authorized_keys}"
chmod 0600 "${authorized_keys}"

systemctl daemon-reload
backup_url="$(sed -n 's/^BACKUP_DATABASE_URL=//p' /etc/sakiot/production.env | head -n 1)"
age_recipient="$(sed -n 's/^AGE_RECIPIENT=//p' /etc/sakiot/production.env | head -n 1)"
age_key_file="$(sed -n 's/^AGE_KEY_FILE=//p' /etc/sakiot/production.env | head -n 1)"
if [[ -n "${backup_url}" && "${backup_url}" != *replace_me* \
      && "${age_recipient}" == age1* && -r "${age_key_file}" ]]; then
  systemctl enable --now \
    sakiot-db-backup-hourly.timer \
    sakiot-db-backup-nightly.timer \
    sakiot-db-restore-test.timer
else
  echo "backup timers installed but not enabled; configure production backup credentials and age key"
fi
echo "production + staging skeleton installed"
echo "edit /etc/sakiot/production.env before the first tag"
echo "edit /etc/sakiot/staging.env and run 'createdb sakiot_staging' before the first main push"

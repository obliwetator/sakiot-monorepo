#!/usr/bin/env bash
set -Eeuo pipefail
umask 027

usage() {
  echo "usage: sudo $0 /absolute/path/to/legacy/data" >&2
  exit 2
}

[[ "${EUID}" -eq 0 ]] || {
  echo "run as root" >&2
  exit 1
}
[[ "$#" -eq 1 ]] || usage

legacy_data="$(realpath -e "$1")"
data_dir="${SAKIOT_DATA_DIR:-/var/lib/sakiot/data}"
state_dir="${SAKIOT_DEPLOY_STATE_DIR:-/var/lib/sakiot/deploy}"
fstab="${SAKIOT_FSTAB:-/etc/fstab}"

[[ "${legacy_data}" = /* && "${data_dir}" = /* ]] || usage
[[ "${legacy_data}" != "${data_dir}" ]] || {
  echo "legacy and production data directories must differ" >&2
  exit 1
}
[[ "${legacy_data}" != *[[:space:]]* && "${data_dir}" != *[[:space:]]* ]] || {
  echo "data directory paths containing whitespace are unsupported" >&2
  exit 1
}

for command in awk curl find findmnt install jq mount mountpoint realpath rsync \
  setfacl systemctl umount; do
  command -v "${command}" >/dev/null 2>&1 || {
    echo "required command not found: ${command}" >&2
    exit 1
  }
done

for directory in voice_recordings no_silence_voice_recordings waveform_data clips; do
  [[ -d "${legacy_data}/${directory}" ]] || {
    echo "legacy data directory missing: ${legacy_data}/${directory}" >&2
    exit 1
  }
done

if [[ ! -e "${data_dir}" ]]; then
  install -d -o sakiot -g sakiot -m 0755 "${data_dir}"
fi
[[ -d "${data_dir}" ]] || {
  echo "production data path is not a directory: ${data_dir}" >&2
  exit 1
}

bot_unit="$(cat "${state_dir}/current-bot.unit")"
[[ "${bot_unit}" =~ ^sakiot-fbi-agent@[A-Za-z0-9._-]+\.service$ ]] || {
  echo "invalid current FBI Agent unit: ${bot_unit}" >&2
  exit 1
}

web_was_active=0
bot_was_active=0
systemctl is-active --quiet sakiot-web.service && web_was_active=1
systemctl is-active --quiet "${bot_unit}" && bot_was_active=1

mounted_by_script=0
fstab_backup=""
completed=0

restore_on_exit() {
  local status=$?

  trap - EXIT INT TERM
  set +e
  if [[ "${completed}" != "1" ]]; then
    systemctl stop sakiot-web.service "${bot_unit}" >/dev/null 2>&1
    if [[ "${mounted_by_script}" == "1" ]]; then
      umount "${data_dir}"
    fi
    if [[ -n "${fstab_backup}" && -f "${fstab_backup}" ]]; then
      cp -a "${fstab_backup}" "${fstab}"
    fi
    [[ "${bot_was_active}" == "1" ]] && systemctl start "${bot_unit}"
    [[ "${web_was_active}" == "1" ]] && systemctl start sakiot-web.service
  fi
  [[ -n "${fstab_backup}" ]] && rm -f "${fstab_backup}"
  exit "${status}"
}
trap restore_on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if mountpoint -q "${data_dir}"; then
  mounted_root="$(findmnt -n -o FSROOT --mountpoint "${data_dir}")"
  [[ "${mounted_root}" == "${legacy_data}" ]] || {
    echo "${data_dir} is already a mount point from ${mounted_root}" >&2
    exit 1
  }
else
  systemctl stop sakiot-web.service "${bot_unit}"

  # Preserve recordings created after the production cutover.
  rsync -a --ignore-existing "${data_dir}/" "${legacy_data}/"

  # Keep legacy ownership while granting the production account full access.
  setfacl -R -m u:sakiot:rwX "${legacy_data}"
  find "${legacy_data}" -type d -exec setfacl -m d:u:sakiot:rwx {} +

  fstab_backup="$(mktemp)"
  cp -a "${fstab}" "${fstab_backup}"
  if awk -v target="${data_dir}" \
    '$1 !~ /^#/ && $2 == target { found=1 } END { exit !found }' "${fstab}"; then
    echo "${fstab} already contains a mount for ${data_dir}" >&2
    exit 1
  fi
  printf '\n# Temporary Sakiot legacy data bind; remove after data migration.\n' >>"${fstab}"
  printf '%s %s none bind,nofail 0 0\n' "${legacy_data}" "${data_dir}" >>"${fstab}"
  findmnt --verify --tab-file "${fstab}" >/dev/null

  mount --bind "${legacy_data}" "${data_dir}"
  mounted_by_script=1
fi

systemctl start "${bot_unit}"
systemctl start sakiot-web.service
systemctl is-active --quiet "${bot_unit}"
systemctl is-active --quiet sakiot-web.service

web_ready=0
for _ in $(seq 1 30); do
  if curl -fsS --connect-timeout 2 --max-time 5 \
    http://127.0.0.1:8900/healthz \
    | jq -e '.status == "ok" and .database == "ready"' >/dev/null; then
    web_ready=1
    break
  fi
  sleep 1
done
[[ "${web_ready}" == "1" ]] || {
  echo "web server failed health check after legacy data mount" >&2
  exit 1
}

completed=1
[[ -n "${fstab_backup}" ]] && rm -f "${fstab_backup}"

echo "legacy data active at ${data_dir}"
echo "source retained at ${legacy_data}; no 61G copy performed"

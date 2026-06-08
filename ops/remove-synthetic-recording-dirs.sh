#!/usr/bin/env bash
set -euo pipefail

[[ "${EUID}" -eq 0 ]] || {
  echo "run as root" >&2
  exit 1
}

env_file="${SAKIOT_ENV_FILE:-/etc/sakiot/production.env}"
[[ -r "${env_file}" ]] || {
  echo "cannot read ${env_file}" >&2
  exit 1
}

set -a
# shellcheck disable=SC1090
source "${env_file}"
set +a

: "${DATABASE_URL:?DATABASE_URL missing from ${env_file}}"
data_dir="${SAKIOT_DATA_DIR:-/var/lib/sakiot/data}"
recording_dir="${data_dir}/voice_recordings"
[[ -d "${recording_dir}" ]] || {
  echo "recording directory missing: ${recording_dir}" >&2
  exit 1
}

removed=0
while IFS= read -r -d '' directory; do
  guild_id="$(basename "${directory}")"
  [[ "${guild_id}" =~ ^9000[0-9]{9}$ ]] || {
    echo "refusing unexpected directory: ${directory}" >&2
    exit 1
  }

  referenced="$(
    psql "${DATABASE_URL}" -Atv ON_ERROR_STOP=1 \
      -c "SELECT EXISTS (
            SELECT 1 FROM audio_files WHERE guild_id = ${guild_id}
          );"
  )"
  [[ "${referenced}" == "f" ]] || {
    echo "refusing database-referenced directory: ${directory}" >&2
    exit 1
  }

  rm -rf -- "${directory}"
  printf 'removed %s\n' "${directory}"
  removed=$((removed + 1))
done < <(
  find "${recording_dir}" -mindepth 1 -maxdepth 1 -type d \
    -name '9000?????????' -print0
)

echo "removed ${removed} synthetic recording directories"

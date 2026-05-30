#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_dir}"

grpcurl_bin="${GRPCURL_BIN:-}"
if [[ -z "${grpcurl_bin}" ]]; then
  grpcurl_bin="$(command -v grpcurl || true)"
fi
if [[ -z "${grpcurl_bin}" && -x "${HOME}/go/bin/grpcurl" ]]; then
  grpcurl_bin="${HOME}/go/bin/grpcurl"
fi
if [[ -z "${grpcurl_bin}" ]]; then
  echo "grpcurl not found. Set GRPCURL_BIN=/path/to/grpcurl" >&2
  exit 1
fi

release_id_for_unit() {
  local unit="$1"
  local release_id="${unit#fbi-agent@}"
  release_id="${release_id%.service}"
  printf '%s\n' "${release_id}"
}

env_file_for_release() {
  local release_id="$1"
  printf '%s\n' "${repo_dir}/releases/${release_id}/service.env"
}

grpc_addr_for_env_file() {
  local env_file="$1"
  awk -F= '$1 == "GRPC_ADDR" { print $2 }' "${env_file}" 2>/dev/null | tail -1
}

env_value_for_env_file() {
  local env_file="$1"
  local key="$2"
  awk -F= -v key="${key}" '$1 == key { print $2 }' "${env_file}" 2>/dev/null | tail -1
}

database_url() {
  if [[ -n "${DATABASE_URL:-}" ]]; then
    printf '%s\n' "${DATABASE_URL}"
    return
  fi
  if [[ -f "${repo_dir}/.env" ]]; then
    local value
    value="$(awk -F= '$1 == "DATABASE_URL" { print substr($0, index($0, "=") + 1) }' "${repo_dir}/.env" | tail -1)"
    value="${value%\"}"
    value="${value#\"}"
    printf '%s\n' "${value}"
  fi
}

delete_voice_leases_for_instance() {
  local instance_id="$1"
  local db_url
  db_url="$(database_url)"
  if [[ -z "${db_url}" ]]; then
    echo "DATABASE_URL not found; cannot delete stale voice leases for ${instance_id}" >&2
    return 0
  fi
  if ! command -v psql >/dev/null 2>&1; then
    echo "psql not found; cannot delete stale voice leases for ${instance_id}" >&2
    return 0
  fi
  psql "${db_url}" \
    --set=ON_ERROR_STOP=1 \
    --set=instance_id="${instance_id}" \
    >/dev/null <<'SQL'
DELETE FROM voice_session_leases WHERE owner_instance_id = :'instance_id';
UPDATE audio_files
   SET end_ts = COALESCE(end_ts, start_ts),
       reaped = CASE WHEN end_ts IS NULL THEN TRUE ELSE reaped END,
       recording_heartbeat_at = NULL
 WHERE recording_owner_instance_id = :'instance_id'
   AND end_ts IS NULL;
SQL
  echo "deleted stale voice leases and closed stale recordings for ${instance_id}"
}

unit_role_label() {
  local unit="$1"
  local active_state unit_file_state
  active_state="$(systemctl --user show "${unit}" --property=ActiveState --value 2>/dev/null || true)"
  unit_file_state="$(systemctl --user show "${unit}" --property=UnitFileState --value 2>/dev/null || true)"

  if [[ "${active_state}" == "active" && "${unit_file_state}" == "enabled" ]]; then
    printf 'current'
  elif [[ "${active_state}" == "active" ]]; then
    printf 'draining'
  elif [[ "${unit_file_state}" == "enabled" ]]; then
    printf 'enabled'
  else
    printf 'old'
  fi
}

unit_description() {
  local unit="$1"
  local release_id="$2"
  local grpc_addr="$3"
  local role active_state unit_file_state active_enter
  role="$(unit_role_label "${unit}")"
  active_state="$(systemctl --user show "${unit}" --property=ActiveState --value 2>/dev/null || true)"
  unit_file_state="$(systemctl --user show "${unit}" --property=UnitFileState --value 2>/dev/null || true)"
  active_enter="$(systemctl --user show "${unit}" --property=ActiveEnterTimestamp --value 2>/dev/null || true)"

  printf '%s, %s, %s, id %s' "${role}" "${active_state:-unknown}" "${unit_file_state:-unknown}" "${release_id}"
  if [[ -n "${grpc_addr}" ]]; then
    printf ', grpc %s' "${grpc_addr}"
  fi
  if [[ -n "${active_enter}" && "${active_enter}" != "n/a" ]]; then
    printf ', since %s' "${active_enter}"
  fi
}

mapfile -t units < <(systemctl --user list-units 'fbi-agent@*.service' --state=active --no-legend --no-pager | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /\.service$/) { print $i; break } }')
if [[ "${#units[@]}" -eq 0 ]]; then
  echo "no active fbi-agent release units found"
  exit 0
fi

echo "Active fbi-agent release units:"
for i in "${!units[@]}"; do
  unit="${units[$i]}"
  release_id="$(release_id_for_unit "${unit}")"
  env_file="$(env_file_for_release "${release_id}")"
  grpc_addr="$(grpc_addr_for_env_file "${env_file}")"
  printf '%d) %s - %s\n' "$((i + 1))" "${unit}" "$(unit_description "${unit}" "${release_id}" "${grpc_addr}")"
done

read -r -p "Force stop which unit number? " choice
if ! [[ "${choice}" =~ ^[0-9]+$ ]] || (( choice < 1 || choice > ${#units[@]} )); then
  echo "invalid choice" >&2
  exit 1
fi

unit="${units[$((choice - 1))]}"
release_id="$(release_id_for_unit "${unit}")"
env_file="$(env_file_for_release "${release_id}")"
grpc_addr="$(grpc_addr_for_env_file "${env_file}")"
instance_id="$(env_value_for_env_file "${env_file}" "BOT_INSTANCE_ID")"
if [[ -z "${grpc_addr}" ]]; then
  echo "missing GRPC_ADDR in ${env_file}" >&2
  exit 1
fi
if [[ -z "${instance_id}" ]]; then
  echo "missing BOT_INSTANCE_ID in ${env_file}" >&2
  exit 1
fi

role="$(unit_role_label "${unit}")"
echo "Selected: ${unit} - $(unit_description "${unit}" "${release_id}" "${grpc_addr}")"
if [[ "${role}" == "current" ]]; then
  echo "WARNING: selected unit looks like the current active release, not a draining old release." >&2
fi

"${grpcurl_bin}" -plaintext \
  -import-path "${repo_dir}/../sakiot-proto/proto" \
  -proto fbi_agent.proto \
  "${grpc_addr}" \
  fbi_agent.Admin/GetDrainStatus || true

read -r -p "Type FORCE to bypass drain and stop ${unit}: " confirm
if [[ "${confirm}" != "FORCE" ]]; then
  echo "aborted"
  exit 0
fi

"${grpcurl_bin}" -plaintext \
  -import-path "${repo_dir}/../sakiot-proto/proto" \
  -proto fbi_agent.proto \
  -d '{"reason":"interactive force stop"}' \
  "${grpc_addr}" \
  fbi_agent.Admin/ForceShutdown || true

delete_voice_leases_for_instance "${instance_id}" || true

for _ in {1..10}; do
  state="$(systemctl --user is-active "${unit}" || true)"
  if [[ "${state}" != "active" ]]; then
    systemctl --user reset-failed "${unit}" || true
    echo "${unit} stopped (${state})"
    exit 0
  fi
  sleep 1
done

echo "${unit} still active after gRPC force shutdown."
read -r -p "Type KILL to SIGKILL ${unit}: " kill_confirm
if [[ "${kill_confirm}" == "KILL" ]]; then
  systemctl --user kill --signal=SIGKILL --kill-who=all "${unit}"
  systemctl --user reset-failed "${unit}" || true
  echo "${unit} killed"
else
  echo "left running"
fi

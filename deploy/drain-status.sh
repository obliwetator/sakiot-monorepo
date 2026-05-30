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

units="$(systemctl --user list-units 'fbi-agent@*.service' --state=active --no-legend --no-pager | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /\.service$/) { print $i; break } }')"
if [[ -z "${units}" ]]; then
  echo "no active fbi-agent release units found"
  exit 0
fi

for unit in ${units}; do
  release_id="${unit#fbi-agent@}"
  release_id="${release_id%.service}"
  env_file="${repo_dir}/releases/${release_id}/service.env"
  state="$(systemctl --user is-active "${unit}" || true)"

  echo "== ${unit} (${state}) =="
  if [[ ! -f "${env_file}" ]]; then
    echo "missing env file: ${env_file}"
    continue
  fi

  grpc_addr="$(awk -F= '$1 == "GRPC_ADDR" { print $2 }' "${env_file}" | tail -1)"
  if [[ -z "${grpc_addr}" ]]; then
    echo "missing GRPC_ADDR in ${env_file}"
    continue
  fi

  "${grpcurl_bin}" -plaintext \
    -import-path "${repo_dir}/../sakiot-proto/proto" \
    -proto fbi_agent.proto \
    "${grpc_addr}" \
    fbi_agent.Admin/GetDrainStatus || true
done

#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_dir}"

release_id="${1:-$(date -u +%Y%m%d%H%M%S)}"
release_dir="${repo_dir}/releases/${release_id}"
release_bin="${release_dir}/fbi_agent"
deploy_state_dir="${repo_dir}/releases/.deploy"
current_grpc_file="${deploy_state_dir}/current-grpc-addr"
old_grpc_addr="${FBI_AGENT_GRPC_ADDR:-$(cat "${current_grpc_file}" 2>/dev/null || printf '127.0.0.1:50052')}"
web_server_url="${WEB_SERVER_URL:-http://127.0.0.1:8900}"
new_service="fbi-agent@${release_id}.service"
user_unit_dir="${HOME}/.config/systemd/user"
user_unit="${user_unit_dir}/fbi-agent@.service"
grpcurl_bin="${GRPCURL_BIN:-}"
if [[ -z "${grpcurl_bin}" ]]; then
  grpcurl_bin="$(command -v grpcurl || true)"
fi
if [[ -z "${grpcurl_bin}" && -x "${HOME}/go/bin/grpcurl" ]]; then
  grpcurl_bin="${HOME}/go/bin/grpcurl"
fi
curl_bin="${CURL_BIN:-}"
if [[ -z "${curl_bin}" ]]; then
  curl_bin="$(command -v curl || true)"
fi
new_grpc_port="$(python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
    s.bind(("127.0.0.1", 0))
    print(s.getsockname()[1])
PY
)"
new_grpc_addr="127.0.0.1:${new_grpc_port}"

if systemctl --user is-active --quiet "${new_service}"; then
  echo "release service already running: ${new_service}" >&2
  exit 1
fi

if [[ -e "${release_dir}" ]]; then
  echo "release directory already exists: ${release_dir}" >&2
  echo "choose a new release id" >&2
  exit 1
fi

install -d -m 0755 "${release_dir}"
install -d -m 0755 "${deploy_state_dir}"

cargo build --release
install -m 0755 target/release/fbi_agent "${release_bin}"
cat > "${release_dir}/service.env" <<EOF
BOT_ROLE=active
BOT_INSTANCE_ID=$(hostname)-${release_id}
RELEASE_ID=${release_id}
GRPC_ADDR=${new_grpc_addr}
DRAIN_TIMEOUT_SECONDS=0
EOF

install -d -m 0755 "${user_unit_dir}"
install -m 0644 deploy/systemd/user/fbi-agent@.service "${user_unit}"
systemctl --user daemon-reload

if [[ -n "${grpcurl_bin}" ]]; then
  "${grpcurl_bin}" -plaintext \
    -import-path proto \
    -proto helloworld.proto \
    -d '{"reason":"deploy '"${release_id}"'"}' \
    "${old_grpc_addr}" \
    helloworld.Admin/StartDrain || true
else
  echo "grpcurl not found; old instance will not be switched to drain mode" >&2
fi

systemctl --user start "${new_service}"
systemctl --user enable "${new_service}" >/dev/null
printf '%s\n' "${new_grpc_addr}" > "${current_grpc_file}"

if [[ -n "${curl_bin}" ]]; then
  "${curl_bin}" -fsS \
    -H 'Content-Type: application/json' \
    -d '{"active":"'"${new_grpc_addr}"'","draining":["'"${old_grpc_addr}"'"]}' \
    "${web_server_url}/internal/fbi-agent/grpc-endpoints" >/dev/null \
    || echo "web server gRPC endpoint registration failed" >&2
else
  echo "curl not found; web server gRPC endpoint registration skipped" >&2
fi

if [[ -n "${grpcurl_bin}" ]]; then
  "${grpcurl_bin}" -plaintext \
    -import-path proto \
    -proto helloworld.proto \
    -d '{"reason":"new release '"${release_id}"' started"}' \
    "${old_grpc_addr}" \
    helloworld.Admin/ShutdownWhenEmpty || true
fi

shopt -s nullglob
for enabled_release in "${user_unit_dir}/default.target.wants"/fbi-agent@*.service; do
  enabled_service="$(basename "${enabled_release}")"
  if [[ "${enabled_service}" != "${new_service}" ]]; then
    systemctl --user disable "${enabled_service}" >/dev/null || true
  fi
done

echo "started ${new_service}"
echo "release binary: ${release_bin}"
echo "new gRPC addr: ${new_grpc_addr}"
echo "old instance drain request sent to ${old_grpc_addr}"

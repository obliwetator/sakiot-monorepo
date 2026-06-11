#!/usr/bin/env bash
set -euo pipefail

# Refreshes the out-of-band deploy framework on the VPS: the ops/ scripts
# under /usr/local/lib/sakiot-deploy and the Rust deploy engine binary.
# Run as root from a monorepo checkout after changing ops/. Unlike
# install-production.sh this never touches users, env files, data dirs,
# or authorized_keys.

if [[ "${EUID}" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
install_root="/usr/local/lib/sakiot-deploy"

# Run the suites as the sakiot user: frontend_publish_test.sh depends on
# write-permission checks that are vacuously true for root.
if [[ -x "${script_dir}/tests/run.sh" ]]; then
  echo "running deploy framework tests"
  sudo -u sakiot "${script_dir}/tests/run.sh"
fi

echo "building sakiot-deploy engine"
if ! sudo -u sakiot bash -lc 'command -v cargo' >/dev/null 2>&1; then
  echo "error: cargo is not available for the sakiot user" >&2
  exit 1
fi
sudo -u sakiot bash -lc \
  "cd '${repo_root}' && CARGO_TARGET_DIR=/var/cache/sakiot/cargo-target \
   cargo build --release --locked --package sakiot-deploy"

echo "installing framework to ${install_root}"
rm -rf "${install_root}"
install -d -o root -g root -m 0755 "${install_root}"
cp -a "${script_dir}/." "${install_root}/"
rm -rf "${install_root}/sakiot-deploy"
install -d -o root -g root -m 0755 "${install_root}/backup"
cp -a "${script_dir}/../sakiot-db/ops/backup/." "${install_root}/backup/"
cp -a "${script_dir}/../sakiot-db/migrations" "${install_root}/backup/migrations"
chown -R root:root "${install_root}"
find "${install_root}" -type d -exec chmod 0755 {} +
find "${install_root}" -type f -name '*.sh' -exec chmod 0755 {} +
chmod 0755 "${install_root}/deploy" "${install_root}/ssh/forced-command"
chmod 0755 "${install_root}/systemctl-wrapper"

install -d -o root -g root -m 0755 "${install_root}/bin"
install -o root -g root -m 0755 \
  /var/cache/sakiot/cargo-target/release/sakiot-deploy \
  "${install_root}/bin/sakiot-deploy"

visudo -cf "${script_dir}/sudoers/sakiot-deploy"
install -m 0440 "${script_dir}/sudoers/sakiot-deploy" /etc/sudoers.d/sakiot-deploy

echo "deploy framework updated"
echo "engine selection: SAKIOT_DEPLOY_ENGINE in /etc/sakiot/{production,staging}.env (rust|bash)"

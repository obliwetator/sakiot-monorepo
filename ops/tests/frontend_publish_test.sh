#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary="$(mktemp -d)"
cleanup() {
  chmod -R u+w "${temporary}" 2>/dev/null || true
  rm -rf "${temporary}"
}
trap cleanup EXIT
mkdir -p "${temporary}/bin" "${temporary}/dist/assets" "${temporary}/target"
chmod 0750 "${temporary}/target"
mkdir -p "${temporary}/target/assets"
chmod 0750 "${temporary}/target/assets"
touch "${temporary}/target/assets/old-hash.js"
chmod 0640 "${temporary}/target/assets/old-hash.js"
touch "${temporary}/dist/assets/app-hash.js" \
  "${temporary}/dist/index.html" \
  "${temporary}/dist/version.json" \
  "${temporary}/dist/favicon.svg"

cat >"${temporary}/bin/rsync" <<'EOF'
#!/usr/bin/env bash
printf 'rsync %s\n' "$*" >>"${CALL_LOG}"
EOF
cat >"${temporary}/bin/install" <<'EOF'
#!/usr/bin/env bash
printf 'install %s\n' "$*" >>"${CALL_LOG}"
cp "${@: -2:1}" "${@: -1}"
EOF
chmod +x "${temporary}/bin/rsync" "${temporary}/bin/install"

CALL_LOG="${temporary}/calls" \
PATH="${temporary}/bin:${PATH}" \
SAKIOT_FRONTEND_ROOT="${temporary}/target" \
SAKIOT_FRONTEND_DIST="${temporary}/dist" \
  "${test_dir}/../../sakiot_stage/scripts/deploy.sh"

mapfile -t calls <"${temporary}/calls"
[[ "${calls[0]}" == rsync*"--no-owner --no-group --no-perms --no-times"*"dist/assets/"* ]]
[[ "${calls[1]}" == rsync*"--exclude=assets.legacy-*/"*"--exclude=index.html"*"dist/"* ]]
[[ "${calls[2]}" == install*"index.html"* ]]
[[ "${calls[3]}" == install*"version.json"* ]]
[[ -f "${temporary}/target/index.html" ]]
[[ -f "${temporary}/target/version.json" ]]
[[ "$(stat -c '%a' "${temporary}/target")" == "755" ]]
[[ "$(stat -c '%a' "${temporary}/target/assets")" == "755" ]]
[[ "$(stat -c '%a' "${temporary}/target/assets/old-hash.js")" == "644" ]]

mkdir -p "${temporary}/legacy-target/assets"
touch "${temporary}/legacy-target/assets/old-hash.js"
chmod 0555 "${temporary}/legacy-target/assets"
: >"${temporary}/calls"

CALL_LOG="${temporary}/calls" \
PATH="${temporary}/bin:${PATH}" \
SAKIOT_FRONTEND_ROOT="${temporary}/legacy-target" \
SAKIOT_FRONTEND_DIST="${temporary}/dist" \
  "${test_dir}/../../sakiot_stage/scripts/deploy.sh"

mapfile -t calls <"${temporary}/calls"
[[ "${calls[0]}" == rsync*"assets.legacy-"*"legacy-target/assets/"* ]]
[[ "${calls[1]}" == rsync*"dist/assets/"*"legacy-target/assets/"* ]]
[[ -d "${temporary}/legacy-target/assets" ]]
compgen -G "${temporary}/legacy-target/assets.legacy-*" >/dev/null

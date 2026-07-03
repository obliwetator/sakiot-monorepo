#!/usr/bin/env bash
set -euo pipefail

# The rescue cases rely on 0555 directories and 0444 files being non-writable,
# which is never true for root. (The foreign-ownership leg of the rescue
# trigger cannot be simulated without root; these cases exercise the
# not-writable leg of the same predicate.)
if [[ "${EUID}" -eq 0 ]]; then
  echo "frontend_publish_test must not run as root" >&2
  exit 1
fi

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
touch "${temporary}/target/favicon.svg"
chmod 0640 "${temporary}/target/favicon.svg"
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
  "${test_dir}/../../sakiot-stage/scripts/deploy.sh"

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
[[ "$(stat -c '%a' "${temporary}/target/favicon.svg")" == "644" ]]

mkdir -p "${temporary}/legacy-target/assets"
touch "${temporary}/legacy-target/assets/old-hash.js"
chmod 0555 "${temporary}/legacy-target/assets"
: >"${temporary}/calls"

CALL_LOG="${temporary}/calls" \
PATH="${temporary}/bin:${PATH}" \
SAKIOT_FRONTEND_ROOT="${temporary}/legacy-target" \
SAKIOT_FRONTEND_DIST="${temporary}/dist" \
  "${test_dir}/../../sakiot-stage/scripts/deploy.sh"

mapfile -t calls <"${temporary}/calls"
[[ "${calls[0]}" == rsync*"assets.legacy-"*"legacy-target/assets/"* ]]
[[ "${calls[1]}" == rsync*"dist/assets/"*"legacy-target/assets/"* ]]
[[ -d "${temporary}/legacy-target/assets" ]]
compgen -G "${temporary}/legacy-target/assets.legacy-*" >/dev/null

# The v1.0.7 wedge: a writable tree containing files the deploy user cannot
# control (0444 stands in for root-owned). The assets rescue must trigger,
# stale top-level files must be reclaimed, index.html must stay present, and
# the final chmod must not touch the moved-aside legacy dir.
mkdir -p "${temporary}/wedged-target/assets"
touch "${temporary}/wedged-target/assets/rooted-hash.js"
chmod 0444 "${temporary}/wedged-target/assets/rooted-hash.js"
touch "${temporary}/wedged-target/stale.css"
chmod 0444 "${temporary}/wedged-target/stale.css"
touch "${temporary}/wedged-target/index.html"
chmod 0444 "${temporary}/wedged-target/index.html"
: >"${temporary}/calls"

CALL_LOG="${temporary}/calls" \
PATH="${temporary}/bin:${PATH}" \
SAKIOT_FRONTEND_ROOT="${temporary}/wedged-target" \
SAKIOT_FRONTEND_DIST="${temporary}/dist" \
  "${test_dir}/../../sakiot-stage/scripts/deploy.sh"

mapfile -t calls <"${temporary}/calls"
[[ "${calls[0]}" == rsync*"assets.legacy-"*"wedged-target/assets/"* ]]
[[ "${calls[1]}" == rsync*"dist/assets/"*"wedged-target/assets/"* ]]
wedged_legacy="$(compgen -G "${temporary}/wedged-target/assets.legacy-*")"
[[ "$(stat -c '%a' "${wedged_legacy}/rooted-hash.js")" == "444" ]]
[[ "$(stat -c '%a' "${temporary}/wedged-target/assets")" == "755" ]]
[[ ! -e "${temporary}/wedged-target/stale.css" ]]
[[ -f "${temporary}/wedged-target/index.html" ]]
[[ "$(stat -c '%a' "${temporary}/wedged-target/index.html")" == "644" ]]

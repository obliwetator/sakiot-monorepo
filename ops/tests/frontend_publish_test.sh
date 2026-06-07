#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
mkdir -p "${temporary}/bin" "${temporary}/dist/assets" "${temporary}/target"
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
EOF
chmod +x "${temporary}/bin/rsync" "${temporary}/bin/install"

CALL_LOG="${temporary}/calls" \
PATH="${temporary}/bin:${PATH}" \
SAKIOT_FRONTEND_ROOT="${temporary}/target" \
SAKIOT_FRONTEND_DIST="${temporary}/dist" \
  "${test_dir}/../../sakiot_stage/scripts/deploy.sh"

mapfile -t calls <"${temporary}/calls"
[[ "${calls[0]}" == rsync*"dist/assets/"* ]]
[[ "${calls[1]}" == rsync*"--exclude=index.html"*"dist/"* ]]
[[ "${calls[2]}" == install*"index.html"* ]]
[[ "${calls[3]}" == install*"version.json"* ]]

#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

mkdir -p "${temporary}/ops/ssh"
cp "${test_dir}/../ssh/forced-command" "${temporary}/ops/ssh/forced-command"
cat >"${temporary}/ops/deploy" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*"
EOF
chmod +x "${temporary}/ops/ssh/forced-command" "${temporary}/ops/deploy"

sha="0123456789abcdef0123456789abcdef01234567"
actual="$(
  SSH_ORIGINAL_COMMAND="release v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${actual}" == "release v1.2.3 ${sha}" ]]

staging_actual="$(
  SSH_ORIGINAL_COMMAND="staging ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_actual}" == "stage ${sha}" ]]

staging_ci_actual="$(
  SSH_ORIGINAL_COMMAND="staging-ci ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_ci_actual}" == "stage-ci ${sha}" ]]

staging_prepare_actual="$(
  SSH_ORIGINAL_COMMAND="staging-ci ${sha} prepare v1.2.3" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_prepare_actual}" == "stage-ci ${sha} --prepare-production v1.2.3" ]]

release_ci_actual="$(
  SSH_ORIGINAL_COMMAND="release-ci v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${release_ci_actual}" == "release-ci v1.2.3 ${sha}" ]]

release_promoted_actual="$(
  SSH_ORIGINAL_COMMAND="release-promoted v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${release_promoted_actual}" == "release-promoted v1.2.3 ${sha}" ]]

if SSH_ORIGINAL_COMMAND="staging ${sha}; id" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted shell metacharacters in staging verb" >&2
  exit 1
fi

if SSH_ORIGINAL_COMMAND="staging main" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted non-sha staging ref" >&2
  exit 1
fi

if SSH_ORIGINAL_COMMAND="release v1.2.3 ${sha}; id" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted shell metacharacters" >&2
  exit 1
fi

if SSH_ORIGINAL_COMMAND="bash" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted interactive shell" >&2
  exit 1
fi

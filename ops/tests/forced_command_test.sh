#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

mkdir -p "${temporary}/ops/ssh"
cp "${test_dir}/../ssh/forced-command" "${temporary}/ops/ssh/forced-command"
cat >"${temporary}/ops/deploy" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "${SAKIOT_GITHUB_TOKEN_STDIN:-0}" "$*"
EOF
chmod +x "${temporary}/ops/ssh/forced-command" "${temporary}/ops/deploy"

mkdir -p "${temporary}/bin"
cat >"${temporary}/bin/sudo" <<'EOF'
#!/usr/bin/env bash
shift
exec "$@"
EOF
chmod +x "${temporary}/bin/sudo"
cat >"${temporary}/ops/preview-slot.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*"
EOF
chmod +x "${temporary}/ops/preview-slot.sh"

sha="0123456789abcdef0123456789abcdef01234567"
actual="$(
  SSH_ORIGINAL_COMMAND="release v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${actual}" == "0|release v1.2.3 ${sha}" ]]

staging_actual="$(
  SSH_ORIGINAL_COMMAND="staging ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_actual}" == "0|stage ${sha}" ]]

staging_ci_actual="$(
  SSH_ORIGINAL_COMMAND="staging-ci ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_ci_actual}" == "1|stage-ci ${sha}" ]]

staging_prepare_actual="$(
  SSH_ORIGINAL_COMMAND="staging-ci ${sha} prepare v1.2.3" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${staging_prepare_actual}" == "1|stage-ci ${sha} --prepare-production v1.2.3" ]]

release_ci_actual="$(
  SSH_ORIGINAL_COMMAND="release-ci v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${release_ci_actual}" == "1|release-ci v1.2.3 ${sha}" ]]

release_promoted_actual="$(
  SSH_ORIGINAL_COMMAND="release-promoted v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${release_promoted_actual}" == "1|release-promoted v1.2.3 ${sha}" ]]

rollback_actual="$(
  SSH_ORIGINAL_COMMAND="rollback v1.2.3 ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${rollback_actual}" == "1|rollback v1.2.3 ${sha}" ]]

preview_ci_actual="$(
  SSH_ORIGINAL_COMMAND="preview-ci clip-editor ${sha}" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${preview_ci_actual}" == "1|preview-ci clip-editor ${sha}" ]]

preview_up_actual="$(
  PATH="${temporary}/bin:${PATH}" \
  SSH_ORIGINAL_COMMAND="preview-up clip-editor" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${preview_up_actual}" == "clip-editor" ]]

preview_remove_actual="$(
  PATH="${temporary}/bin:${PATH}" \
  SSH_ORIGINAL_COMMAND="preview-remove clip-editor" \
    "${temporary}/ops/ssh/forced-command"
)"
[[ "${preview_remove_actual}" == "clip-editor --remove" ]]

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

if SSH_ORIGINAL_COMMAND="preview-up Bad_Slot" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted invalid slot name in preview-up" >&2
  exit 1
fi

if SSH_ORIGINAL_COMMAND="preview-remove 'x; rm -rf /'" \
  "${temporary}/ops/ssh/forced-command" >/dev/null 2>&1; then
  echo "forced command accepted shell metacharacters in preview-remove" >&2
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

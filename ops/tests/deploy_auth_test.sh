#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

mkdir -p "${temporary}/ops/bin"
cp "${test_dir}/../deploy" "${temporary}/ops/deploy"
cp "${test_dir}/../git-askpass.sh" "${temporary}/ops/git-askpass.sh"
chmod +x "${temporary}/ops/deploy" "${temporary}/ops/git-askpass.sh"

cat >"${temporary}/ops/bin/sakiot-deploy" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

[[ "${GIT_TERMINAL_PROMPT}" == "0" ]]
[[ "${GIT_ASKPASS_REQUIRE}" == "force" ]]
[[ "${GIT_CONFIG_COUNT}" == "2" ]]
[[ "${GIT_CONFIG_KEY_0}" == "protocol.version" ]]
[[ "${GIT_CONFIG_VALUE_0}" == "2" ]]
[[ "${GIT_CONFIG_KEY_1}" == "credential.helper" ]]
[[ -z "${GIT_CONFIG_VALUE_1}" ]]
[[ "$("${GIT_ASKPASS}" "Username for 'https://github.com':")" == "x-access-token" ]]
[[ "$("${GIT_ASKPASS}" "Password for 'https://x-access-token@github.com':")" == "${EXPECTED_TOKEN}" ]]
printf '%s\n' "${SAKIOT_GITHUB_TOKEN_FILE}" >"${TOKEN_PATH_RECORD}"
printf '%s\n' "$*"
EOF
chmod +x "${temporary}/ops/bin/sakiot-deploy"

token="ghs_0123456789abcdefghijklmnopqrstuvwxyz"
actual="$(
  printf '%s\n' "${token}" \
    | EXPECTED_TOKEN="${token}" \
      TOKEN_PATH_RECORD="${temporary}/token-path" \
      SAKIOT_GITHUB_TOKEN_STDIN=1 \
      "${temporary}/ops/deploy" stage-ci \
        0123456789abcdef0123456789abcdef01234567
)"
[[ "${actual}" == "stage-ci 0123456789abcdef0123456789abcdef01234567" ]]
token_path="$(cat "${temporary}/token-path")"
[[ ! -e "${token_path}" ]]

if printf '\n' \
  | SAKIOT_GITHUB_TOKEN_STDIN=1 "${temporary}/ops/deploy" stage-ci \
      0123456789abcdef0123456789abcdef01234567 >/dev/null 2>&1; then
  echo "deploy wrapper accepted an empty GitHub token" >&2
  exit 1
fi

if printf '%s\n%s\n' "${token}" unexpected \
  | SAKIOT_GITHUB_TOKEN_STDIN=1 "${temporary}/ops/deploy" stage-ci \
      0123456789abcdef0123456789abcdef01234567 >/dev/null 2>&1; then
  echo "deploy wrapper accepted extra stdin after GitHub token" >&2
  exit 1
fi

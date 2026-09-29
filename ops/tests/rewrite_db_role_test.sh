#!/usr/bin/env bash
# Moving an env file to another database role rewrites only the credentials of
# DATABASE_URL and BACKUP_DATABASE_URL, and keeps the previous file.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rewrite="${test_dir}/../rewrite-db-role.sh"
work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

fail() {
  echo "rewrite db role: $*" >&2
  exit 1
}

env_file="${work_dir}/staging.env"
cat >"${env_file}" <<'ENV'
# comment mentioning DATABASE_URL=postgres://sakiot:old@nowhere/x
DATABASE_URL=postgres://sakiot:old-secret@127.0.0.1/sakiot_staging?sslmode=disable
SAKIOT_TEST_DATABASE_URL=postgres://sakiot_test:test-secret@127.0.0.1/sakiot_test
BACKUP_DATABASE_URL=postgresql://sakiot:old-secret@127.0.0.1:5432/sakiot_staging
PORT=8901
ENV
chmod 0640 "${env_file}"

"${rewrite}" "${env_file}" sakiot_staging N3w-pass_word >/dev/null

expected="$(cat <<'ENV'
# comment mentioning DATABASE_URL=postgres://sakiot:old@nowhere/x
DATABASE_URL=postgres://sakiot_staging:N3w-pass_word@127.0.0.1/sakiot_staging?sslmode=disable
SAKIOT_TEST_DATABASE_URL=postgres://sakiot_test:test-secret@127.0.0.1/sakiot_test
BACKUP_DATABASE_URL=postgresql://sakiot_staging:N3w-pass_word@127.0.0.1:5432/sakiot_staging
PORT=8901
ENV
)"
[[ "$(cat "${env_file}")" == "${expected}" ]] || fail "unexpected rewrite: $(cat "${env_file}")"
[[ "$(stat -c %a "${env_file}")" == "640" ]] || fail "the env file mode changed"

backups=("${env_file}".bak-*)
[[ "${#backups[@]}" -eq 1 ]] || fail "expected one backup"
grep -q "sakiot:old-secret@" "${backups[0]}" || fail "the backup lost the previous credentials"

if "${rewrite}" "${env_file}" sakiot_staging 'not url safe@' >/dev/null 2>&1; then
  fail "accepted a password that is not URL-safe"
fi
printf 'PORT=1\n' >"${work_dir}/empty.env"
if "${rewrite}" "${work_dir}/empty.env" sakiot_staging pass >/dev/null 2>&1; then
  fail "reported success without a DATABASE_URL to rewrite"
fi
[[ "$(cat "${work_dir}/empty.env")" == "PORT=1" ]] || fail "a failed rewrite modified the file"

# A release copy is rewritten in place, leaving no copy of the old credentials.
release_env="${work_dir}/service.env"
printf 'RELEASE_ID=r1\nDATABASE_URL=postgres://sakiot:old-secret@127.0.0.1/sakiot_preview_zz\n' >"${release_env}"
"${rewrite}" --no-backup "${release_env}" sakiot_slot slot-pass >/dev/null
grep -qx "DATABASE_URL=postgres://sakiot_slot:slot-pass@127.0.0.1/sakiot_preview_zz" "${release_env}" \
  || fail "the release env was not rewritten: $(cat "${release_env}")"
if compgen -G "${release_env}.bak-*" >/dev/null; then
  fail "--no-backup left a copy of the old credentials"
fi

echo "rewrite db role: ok"

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$temporary/bin" "$temporary/backups"
export PATH="$temporary/bin:$PATH"
export CALL_LOG="$temporary/calls.log"
export REMOTE_FIXTURE="$temporary/remote.dump.age"

cat >"$temporary/bin/pg_dump" <<'EOF'
#!/usr/bin/env bash
printf 'encrypted-dump-source\n'
EOF

cat >"$temporary/bin/age" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
out=""
args=("$@")
for ((index = 0; index < ${#args[@]}; index++)); do
  if [[ "${args[$index]}" == "-o" ]]; then
    out="${args[$((index + 1))]}"
  fi
done
if [[ -n "$out" ]]; then
  cat >"$out"
else
  cat "${args[-1]}"
fi
EOF

cat >"$temporary/bin/rclone" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'rclone %s\n' "$*" >>"$CALL_LOG"
[[ "${FAIL_RCLONE:-0}" != 1 ]] || exit 19
case " $* " in
  *" lsf "*) printf 'sakiot_rouvas_nightly_2026-07-16_0317.dump.age\n' ;;
  *" copyto "*) cp "$REMOTE_FIXTURE" "${@: -1}" ;;
  *" copy "*)
    [[ -z "${EXPECT_PRESENT_AT_COPY:-}" || -f "$EXPECT_PRESENT_AT_COPY" ]]
    ;;
esac
EOF

cat >"$temporary/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf 'curl %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$temporary/bin/dropdb" <<'EOF'
#!/usr/bin/env bash
printf 'dropdb %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$temporary/bin/createdb" <<'EOF'
#!/usr/bin/env bash
printf 'createdb %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$temporary/bin/pg_restore" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf 'pg_restore %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$temporary/bin/psql" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  *current_database\(\)*)
    if [[ "$*" == *sakiot_rouvas* ]]; then
      printf 'sakiot_rouvas\n'
    else
      printf 'safe_restore\n'
    fi
    ;;
  *information_schema.tables*) printf '12\n' ;;
  *) printf '1\n' ;;
esac
EOF

cat >"$temporary/bin/sqlx" <<'EOF'
#!/usr/bin/env bash
printf 'installed 20260716000000 media archive\n'
EOF

chmod +x "$temporary/bin"/*
printf 'encrypted-remote-dump\n' >"$REMOTE_FIXTURE"
touch "$temporary/age-key.txt"

env_file="$temporary/test.env"
cat >"$env_file" <<EOF
BACKUP_DATABASE_URL=postgres://test:test@localhost/testdb
BACKUP_DIR=$temporary/backups
AGE_RECIPIENT=age1test
AGE_KEY_FILE=$temporary/age-key.txt
B2_BACKUP_REMOTE=b2:test-backups
RCLONE_CONFIG=$temporary/rclone.conf
HOURLY_RETENTION_DAYS=7
NIGHTLY_RETENTION_DAYS=90
PREMIGRATE_RETENTION_DAYS=90
HEALTHCHECK_URL=https://health.example/success
HEALTHCHECK_URL_RESTORE=https://health.example/restore
EOF
touch "$temporary/rclone.conf"
export SAKIOT_ENV_FILE="$env_file"

old_backup="$temporary/backups/sakiot_rouvas_hourly_2020-01-01_0000.dump.age"
touch "$old_backup"
touch -d '20 days ago' "$old_backup"
export EXPECT_PRESENT_AT_COPY="$old_backup"
"$repo_root/sakiot-db/ops/backup/backup.sh" hourly >/dev/null
[[ ! -e "$old_backup" ]]
grep -q 'rclone .* copy .*--filter + \*.dump.age .*--filter - \*' "$CALL_LOG"
grep -q 'curl .*health.example/success' "$CALL_LOG"
if grep -q 'rclone .* sync ' "$CALL_LOG"; then
  echo 'backup unexpectedly used rclone sync' >&2
  exit 1
fi

: >"$CALL_LOG"
failed_old="$temporary/backups/sakiot_rouvas_hourly_2020-01-02_0000.dump.age"
touch "$failed_old"
touch -d '20 days ago' "$failed_old"
export EXPECT_PRESENT_AT_COPY="$failed_old"
export FAIL_RCLONE=1
if "$repo_root/sakiot-db/ops/backup/backup.sh" hourly >/dev/null 2>&1; then
  echo 'backup unexpectedly succeeded when B2 copy failed' >&2
  exit 1
fi
unset FAIL_RCLONE
[[ -e "$failed_old" ]]
if grep -q '^curl ' "$CALL_LOG"; then
  echo 'failed backup unexpectedly sent success ping' >&2
  exit 1
fi

: >"$CALL_LOG"
"$repo_root/sakiot-db/ops/backup/restore-test.sh" >/dev/null
grep -q 'rclone .* lsf .*b2:test-backups' "$CALL_LOG"
grep -q 'rclone .* copyto .*b2:test-backups/' "$CALL_LOG"
grep -q '^pg_restore ' "$CALL_LOG"
grep -q 'curl .*health.example/restore' "$CALL_LOG"

: >"$CALL_LOG"
if "$repo_root/sakiot-db/ops/backup/restore.sh" \
  "$REMOTE_FIXTURE" 'postgres://test:test@localhost/sakiot_rouvas' >/dev/null 2>&1; then
  echo 'live restore URL unexpectedly bypassed --force guard' >&2
  exit 1
fi
if grep -q '^pg_restore ' "$CALL_LOG"; then
  echo 'refused live restore unexpectedly invoked pg_restore' >&2
  exit 1
fi

"$repo_root/sakiot-db/ops/backup/restore.sh" \
  "$REMOTE_FIXTURE" 'postgres://test:test@localhost/safe_restore' >/dev/null
grep -q '^pg_restore .*safe_restore' "$CALL_LOG"

: >"$CALL_LOG"
"$repo_root/sakiot-db/ops/backup/restore.sh" \
  "$REMOTE_FIXTURE" 'postgres://test:test@localhost/sakiot_rouvas' --force >/dev/null
grep -q '^pg_restore .*sakiot_rouvas' "$CALL_LOG"

echo 'db backup tests: OK'

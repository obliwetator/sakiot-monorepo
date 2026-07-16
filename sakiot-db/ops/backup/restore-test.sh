#!/usr/bin/env bash
#
# Verify backups are actually restorable. An untested backup is not a backup.
#
# Restores the newest nightly dump into a throwaway db, checks the schema came
# back and the migration ledger is clean, prints row counts, then drops the db.
# Meant to run monthly from cron.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MIGRATIONS_DIR="${SAKIOT_MIGRATIONS_DIR:-$REPO_ROOT/migrations}"
# shellcheck source=load-env.sh
source "$SCRIPT_DIR/load-env.sh"
: "${AGE_KEY_FILE:?set AGE_KEY_FILE in the root .env}"
: "${BACKUP_DIR:?set BACKUP_DIR in the root .env}"
: "${BACKUP_DATABASE_URL:?set BACKUP_DATABASE_URL in the root .env}"
: "${B2_BACKUP_REMOTE:?set B2_BACKUP_REMOTE (for example b2:sakiot-db-backups-random) in the root .env}"
RCLONE_CONFIG="${RCLONE_CONFIG:-/etc/sakiot/rclone.conf}"

TESTDB="sakiot_rouvas_restoretest"
base_url="${BACKUP_DATABASE_URL%%\?*}"
query=""
if [[ "$BACKUP_DATABASE_URL" == *\?* ]]; then
  query="?${BACKUP_DATABASE_URL#*\?}"
fi
if [[ "$base_url" != postgres://*/* && "$base_url" != postgresql://*/* ]]; then
  echo "BACKUP_DATABASE_URL must be a PostgreSQL URL" >&2
  exit 1
fi
TEST_DATABASE_URL="${base_url%/*}/${TESTDB}${query}"

# Select and download newest remote nightly, falling back to any label. Dump
# names contain sortable UTC dates/times, so lexical order is chronological.
remote_name="$(rclone --config "$RCLONE_CONFIG" lsf "$B2_BACKUP_REMOTE" \
  --files-only --include 'sakiot_rouvas_nightly_*.dump.age' | sort | tail -1)"
if [[ -z "$remote_name" ]]; then
  remote_name="$(rclone --config "$RCLONE_CONFIG" lsf "$B2_BACKUP_REMOTE" \
    --files-only --include 'sakiot_rouvas_*.dump.age' | sort | tail -1)"
fi
[[ -n "$remote_name" ]] || { echo "no B2 backups found in $B2_BACKUP_REMOTE" >&2; exit 1; }
download_dir="$(mktemp -d)"
newest="$download_dir/$remote_name"
rclone --config "$RCLONE_CONFIG" copyto "$B2_BACKUP_REMOTE/$remote_name" "$newest"
echo "testing restore of downloaded B2 backup: $remote_name"

dropdb --if-exists --maintenance-db="$BACKUP_DATABASE_URL" "$TESTDB"
createdb --maintenance-db="$BACKUP_DATABASE_URL" "$TESTDB"
cleanup() {
  dropdb --if-exists --maintenance-db="$BACKUP_DATABASE_URL" "$TESTDB" >/dev/null 2>&1 || true
  rm -rf "$download_dir"
}
trap cleanup EXIT

age -d -i "$AGE_KEY_FILE" "$newest" \
  | pg_restore --clean --if-exists --no-owner -d "$TEST_DATABASE_URL"

# Schema sanity: tables must have come back.
tables="$(psql -d "$TEST_DATABASE_URL" -tAc \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public';")"
echo "public tables restored: $tables"
[[ "$tables" -gt 0 ]] || { echo "FAIL: no tables restored" >&2; exit 1; }

# Migration ledger: every migration in migrations/ should already be applied
# in the restored db (warn, don't hard-fail — baseline is schema-only).
if command -v sqlx >/dev/null 2>&1 && [[ -d "$MIGRATIONS_DIR" ]]; then
  if DATABASE_URL="$TEST_DATABASE_URL" \
       sqlx migrate info --source "$MIGRATIONS_DIR" 2>/dev/null | grep -qi pending; then
    echo "WARN: restored db reports pending migrations" >&2
  fi
else
  echo "WARN: migration check skipped; sqlx or migrations directory unavailable" >&2
fi

# Row counts (informational — live drifts from the backup snapshot).
echo "row counts (restored db):"
for t in audio_files voice_state_events user_guilds; do
  c="$(psql -d "$TEST_DATABASE_URL" -tAc "SELECT count(*) FROM $t" 2>/dev/null || echo NA)"
  echo "  $t: $c"
done

if [[ -n "${HEALTHCHECK_URL_RESTORE:-}" ]]; then
  curl -fsS -m 10 "$HEALTHCHECK_URL_RESTORE" >/dev/null || true
fi
echo "restore-test OK"

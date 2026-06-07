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
# shellcheck source=backup.env.example
source "$SCRIPT_DIR/backup.env"
: "${AGE_KEY_FILE:?set AGE_KEY_FILE in backup.env}"
: "${BACKUP_DIR:?set BACKUP_DIR in backup.env}"

TESTDB="sakiot_rouvas_restoretest"

# Newest nightly, falling back to newest of any label.
newest="$(ls -1t "$BACKUP_DIR"/sakiot_rouvas_nightly_*.dump.age 2>/dev/null | head -1 || true)"
[[ -z "$newest" ]] && newest="$(ls -1t "$BACKUP_DIR"/sakiot_rouvas_*.dump.age 2>/dev/null | head -1 || true)"
[[ -z "$newest" ]] && { echo "no backups found in $BACKUP_DIR" >&2; exit 1; }
echo "testing restore of: $newest"

dropdb --if-exists "$TESTDB"
createdb "$TESTDB"
trap 'dropdb --if-exists "$TESTDB" >/dev/null 2>&1 || true' EXIT

age -d -i "$AGE_KEY_FILE" "$newest" \
  | pg_restore --clean --if-exists --no-owner -d "$TESTDB"

# Schema sanity: tables must have come back.
tables="$(psql -d "$TESTDB" -tAc \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public';")"
echo "public tables restored: $tables"
[[ "$tables" -gt 0 ]] || { echo "FAIL: no tables restored" >&2; exit 1; }

# Migration ledger: every migration in migrations/ should already be applied
# in the restored db (warn, don't hard-fail — baseline is schema-only).
if DATABASE_URL="postgres://${PGUSER:-postgres}@${PGHOST:-localhost}:${PGPORT:-5432}/$TESTDB" \
     sqlx migrate info --source "$REPO_ROOT/migrations" 2>/dev/null | grep -qi pending; then
  echo "WARN: restored db reports pending migrations" >&2
fi

# Row counts (informational — live drifts from the backup snapshot).
echo "row counts (restored db):"
for t in audio_files voice_state_events user_guilds; do
  c="$(psql -d "$TESTDB" -tAc "SELECT count(*) FROM $t" 2>/dev/null || echo NA)"
  echo "  $t: $c"
done

if [[ -n "${HEALTHCHECK_URL_RESTORE:-}" ]]; then
  curl -fsS -m 10 "$HEALTHCHECK_URL_RESTORE" >/dev/null || true
fi
echo "restore-test OK"

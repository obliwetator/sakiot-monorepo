#!/usr/bin/env bash
#
# Encrypted logical backup of the sakiot_rouvas Postgres database.
#
# Streams `pg_dump -Fc` straight into `age` — no plaintext dump ever touches
# disk. Writes to a `.partial` file first and only renames on success, so a
# failed dump can never be promoted to a "valid" truncated backup.
#
# Usage: backup.sh [hourly|nightly|pre-migrate]   (default: hourly)
#
set -euo pipefail

LABEL="${1:-hourly}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load-env.sh
source "$SCRIPT_DIR/load-env.sh"

: "${BACKUP_DATABASE_URL:?set BACKUP_DATABASE_URL in the root .env}"
: "${BACKUP_DIR:?set BACKUP_DIR in the root .env}"
: "${AGE_RECIPIENT:?set AGE_RECIPIENT (age public key) in the root .env}"
: "${B2_BACKUP_REMOTE:?set B2_BACKUP_REMOTE (for example b2:sakiot-db-backups-random) in the root .env}"
RCLONE_CONFIG="${RCLONE_CONFIG:-/etc/sakiot/rclone.conf}"
HOURLY_RETENTION_DAYS="${HOURLY_RETENTION_DAYS:-7}"
NIGHTLY_RETENTION_DAYS="${NIGHTLY_RETENTION_DAYS:-90}"
PREMIGRATE_RETENTION_DAYS="${PREMIGRATE_RETENTION_DAYS:-90}"

mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"

# Single-flight: never let two backups run at once.
exec 9>"$BACKUP_DIR/.backup.lock"
if ! flock -n 9; then
  echo "another backup is running; aborting" >&2
  exit 1
fi

ts="$(date +%F_%H%M)"
out="$BACKUP_DIR/sakiot_rouvas_${LABEL}_${ts}.dump.age"
tmp="$out.partial"

trap 'rm -f "$tmp"' EXIT

# pipefail + errexit: if pg_dump dies mid-stream the pipeline fails and we exit
# before the rename, leaving only the .partial (which the trap removes).
pg_dump -Fc "$BACKUP_DATABASE_URL" | age -r "$AGE_RECIPIENT" -o "$tmp"

mv "$tmp" "$out"
chmod 600 "$out"
echo "wrote $out ($(du -h "$out" | cut -f1))"

# Offsite durability is part of backup success. Copy every completed encrypted
# dump before any local pruning. `copy` never deletes destination objects;
# `.partial` files are excluded and B2 transfers are checksum-verified by
# rclone. With `set -e`, upload failure suppresses both pruning and success ping.
rclone --config "$RCLONE_CONFIG" copy "$BACKUP_DIR" "$B2_BACKUP_REMOTE" \
  --filter '+ *.dump.age' --filter '- *'
echo "copied completed encrypted dumps to $B2_BACKUP_REMOTE"

# Retention: delete encrypted dumps of each label older than N days.
prune() { # <label> <days>
  find "$BACKUP_DIR" -maxdepth 1 -type f \
    -name "sakiot_rouvas_${1}_*.dump.age" -mtime "+${2}" -print -delete
}
prune hourly      "$HOURLY_RETENTION_DAYS"
prune nightly     "$NIGHTLY_RETENTION_DAYS"
prune pre-migrate "$PREMIGRATE_RETENTION_DAYS"

# Optional dead-man ping (healthchecks.io etc.) so silent cron failures surface.
if [[ -n "${HEALTHCHECK_URL:-}" ]]; then
  curl -fsS -m 10 "$HEALTHCHECK_URL" >/dev/null || true
fi

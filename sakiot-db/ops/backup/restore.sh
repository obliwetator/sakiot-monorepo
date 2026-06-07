#!/usr/bin/env bash
#
# Restore an encrypted dump into a target database.
#
# Usage: restore.sh <file.dump.age> <target_db> [--force]
#   - Refuses to restore over the live db (sakiot_rouvas) without --force.
#   - For a single table:  add `-t <table>` by editing the pg_restore line, or
#     decrypt manually:  age -d -i KEY file.dump.age | pg_restore -t TABLE -d DB
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=load-env.sh
source "$SCRIPT_DIR/load-env.sh"
: "${AGE_KEY_FILE:?set AGE_KEY_FILE (age private key, restore only) in the root .env}"

FILE="${1:?usage: restore.sh <file.dump.age> <target_db> [--force]}"
TARGET="${2:?usage: restore.sh <file.dump.age> <target_db> [--force]}"
FORCE="${3:-}"

[[ -f "$FILE" ]] || { echo "no such file: $FILE" >&2; exit 1; }

if [[ "$TARGET" == "sakiot_rouvas" && "$FORCE" != "--force" ]]; then
  echo "refusing to restore over LIVE db 'sakiot_rouvas' without --force" >&2
  exit 1
fi

echo "restoring $FILE -> $TARGET"
age -d -i "$AGE_KEY_FILE" "$FILE" \
  | pg_restore --clean --if-exists --no-owner -d "$TARGET"
echo "done"

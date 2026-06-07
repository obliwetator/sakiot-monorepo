#!/usr/bin/env bash
#
# Take a labelled backup, then run pending migrations. Use this in prod instead
# of bare `sqlx migrate run` so every schema change has an immediate rollback
# point (restore the `pre-migrate` dump if the migration goes wrong).
#
# Usage: pre-migrate-backup.sh
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

"$SCRIPT_DIR/backup.sh" pre-migrate

echo "running migrations..."
sqlx migrate run --source "$REPO_ROOT/migrations"

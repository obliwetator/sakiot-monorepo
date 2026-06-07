#!/usr/bin/env bash
set -euo pipefail

TARGET="/var/www/patrykstyla.com"
SRC="./dist/"
ASSETS_SRC="${SRC}assets/"
ASSETS_TARGET="${TARGET}/assets/"

if [ ! -d "$SRC" ]; then
  echo "dist/ missing — run 'bun run build' first" >&2
  exit 1
fi

if [ ! -d "$TARGET" ]; then
  echo "Target $TARGET does not exist — refusing to create" >&2
  exit 1
fi

if [ ! -d "$ASSETS_SRC" ]; then
  echo "dist/assets/ missing — run 'bun run build' first" >&2
  exit 1
fi

mkdir -p "$ASSETS_TARGET"

# Keep old hashed assets so stale cached HTML and open browser sessions can still load.
rsync -a --delete --checksum --exclude='assets/' "$SRC" "$TARGET/"
rsync -a --checksum "$ASSETS_SRC" "$ASSETS_TARGET/"

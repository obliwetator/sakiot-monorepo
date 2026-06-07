#!/usr/bin/env bash
set -euo pipefail

TARGET="${SAKIOT_FRONTEND_ROOT:-/var/www/patrykstyla.com}"
SRC="${SAKIOT_FRONTEND_DIST:-./dist}/"
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

if [ -d "$ASSETS_TARGET" ] && [ ! -w "$ASSETS_TARGET" ]; then
  LEGACY_ASSETS="${TARGET}/assets.legacy-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  mv "$ASSETS_TARGET" "$LEGACY_ASSETS"
  mkdir -p "$ASSETS_TARGET"
  rsync -a --no-owner --no-group --checksum "$LEGACY_ASSETS/" "$ASSETS_TARGET/"
fi

mkdir -p "$ASSETS_TARGET"

# Keep old hashed assets so stale cached HTML and open browser sessions can still load.
rsync -a --no-owner --no-group --checksum "$ASSETS_SRC" "$ASSETS_TARGET/"
rsync -a --no-owner --no-group --delete --checksum \
  --exclude='assets/' \
  --exclude='assets.legacy-*/' \
  --exclude='index.html' \
  --exclude='version.json' \
  "$SRC" "$TARGET/"

INDEX_TEMP="${TARGET}/.index.html.new.$$"
VERSION_TEMP="${TARGET}/.version.json.new.$$"
trap 'rm -f "$INDEX_TEMP" "$VERSION_TEMP"' EXIT
install -m 0644 "${SRC}index.html" "$INDEX_TEMP"
mv -f "$INDEX_TEMP" "${TARGET}/index.html"
install -m 0644 "${SRC}version.json" "$VERSION_TEMP"
mv -f "$VERSION_TEMP" "${TARGET}/version.json"

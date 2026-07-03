#!/usr/bin/env bash
set -euo pipefail
umask 022

TARGET="${SAKIOT_FRONTEND_ROOT:-/var/www/patrykstyla.com}"
SRC="${SAKIOT_FRONTEND_DIST:-./dist}/"
ASSETS_SRC="${SRC}assets/"
ASSETS_TARGET="${TARGET}/assets/"

# find(1) test for files this user cannot fully control: not ours (chmod by a
# non-owner fails) or not writable. A manual publish made as root left such
# files behind and wedged the v1.0.7 deploy until a manual chown.
RECLAIM_TEST=( "(" ! -user "$(id -un)" -o ! -writable ")" )

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

chmod 0755 "$TARGET"

# Rebuild the assets dir when we cannot write it or anything inside needs
# reclaiming: the rsync copy back recreates every file owned by this user.
if [ -d "$ASSETS_TARGET" ] && { [ ! -w "$ASSETS_TARGET" ] ||
  [ -n "$(find "$ASSETS_TARGET" "${RECLAIM_TEST[@]}" -print -quit)" ]; }; then
  LEGACY_ASSETS="${TARGET}/assets.legacy-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  mv "$ASSETS_TARGET" "$LEGACY_ASSETS"
  mkdir -p "$ASSETS_TARGET"
  rsync -a --no-owner --no-group --no-perms --no-times --checksum \
    "$LEGACY_ASSETS/" "$ASSETS_TARGET/"
fi

mkdir -p "$ASSETS_TARGET"
chmod 0755 "$ASSETS_TARGET"

# Keep old hashed assets so stale cached HTML and open browser sessions can still load.
rsync -a --no-owner --no-group --no-perms --no-times --checksum \
  "$ASSETS_SRC" "$ASSETS_TARGET/"
chmod -R u=rwX,go=rX "$ASSETS_TARGET"

# Reclaim foreign top-level files before syncing: rsync --checksum skips
# content-identical files, which would leave them unwritable for the final
# chmod. index.html and version.json are rewritten unconditionally below and
# must stay present throughout, so they are left alone here.
find "$TARGET" -maxdepth 1 -type f ! -name index.html ! -name version.json \
  "${RECLAIM_TEST[@]}" -exec rm -f {} +

rsync -a --no-owner --no-group --no-perms --no-times --delete --checksum \
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

# The tree-wide chmod runs last, once every published file is owned by this
# user (a possibly foreign index.html/version.json was just replaced above).
# assets.legacy-* keeps whatever files the old tree had (possibly foreign
# owned); a chmod there would fail and abort the deploy, so prune it.
find "$TARGET" -name 'assets.legacy-*' -prune -o -exec chmod u=rwX,go=rX {} +

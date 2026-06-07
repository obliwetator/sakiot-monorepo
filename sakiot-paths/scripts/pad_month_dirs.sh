#!/usr/bin/env bash
# One-off: pad single-digit month directory names to two digits under the
# recording trees. Safe to re-run: already-padded dirs are skipped.
#
# Layout: <root>/<guild>/<channel>/<year>/<month>/...
# Pads the fifth level only.

set -euo pipefail

roots=(
  "./voice_recordings"
  "./no_silence_voice_recordings"
)

for root in "${roots[@]}"; do
  [[ -d "$root" ]] || { echo "skip $root (absent)"; continue; }

  while IFS= read -r -d '' month_dir; do
    month_name="$(basename "$month_dir")"
    # only single-digit 1..9
    [[ "$month_name" =~ ^[1-9]$ ]] || continue
    parent="$(dirname "$month_dir")"
    padded="$(printf '%02d' "$month_name")"
    target="$parent/$padded"

    if [[ -e "$target" ]]; then
      echo "merge: $month_dir -> $target (already exists, moving contents)"
      # move children, then remove empty source
      shopt -s dotglob nullglob
      for child in "$month_dir"/*; do
        mv -n "$child" "$target/"
      done
      shopt -u dotglob nullglob
      rmdir "$month_dir" || echo "  could not remove $month_dir (non-empty?)"
    else
      echo "rename: $month_dir -> $target"
      mv "$month_dir" "$target"
    fi
  done < <(find "$root" -mindepth 4 -maxdepth 4 -type d -print0)
done

echo "done."

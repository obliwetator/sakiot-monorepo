#!/usr/bin/env bash
# Rename legacy recordings `{ts}-{uid}-{username}` -> `{ts}-{uid}` on disk
# AND update audio_files.file_name PK in postgres. Also pads month dir.
#
# Usage: migrate_strip_username.sh [--apply]
#   default is dry-run.
#
# Reads DATABASE_URL from .env. Expects base dirs relative to pwd:
#   ./voice_recordings            (source of truth; may be a symlink)
#   ./no_silence_voice_recordings
#   ./waveform_data               (present on both web_server and FBI-agent;
#                                  script updates whichever exists at this cwd)

set -euo pipefail

APPLY=0
[[ "${1:-}" == "--apply" ]] && APPLY=1

# --- load DATABASE_URL -------------------------------------------------------
if [[ -f .env ]]; then
  # shellcheck disable=SC1091
  set -o allexport; source .env; set +o allexport
fi
: "${DATABASE_URL:?DATABASE_URL not set (check .env)}"

VR="./voice_recordings"
NVR="./no_silence_voice_recordings"
WD="./waveform_data"

echo "mode: $([[ $APPLY -eq 1 ]] && echo APPLY || echo DRY-RUN)"
echo "cwd:  $(pwd)"
echo

run_sql() {
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -At "$@"
}

# Rows with legacy stem = three+ dash groups, at least one extra segment after
# `{ts}-{uid}`.
SQL_SELECT="SELECT file_name, guild_id, channel_id, year, month
            FROM audio_files
            WHERE file_name ~ '^[0-9]+-[0-9]+-.+';"

total=0; moved=0; skipped=0; missing=0; conflicts=0

while IFS=$'\t' read -r stem guild channel year month; do
  [[ -z "${stem:-}" ]] && continue
  total=$((total+1))

  new_stem="${stem%%-*}-$(echo "$stem" | cut -d- -f2)"
  [[ "$new_stem" == "$stem" ]] && { skipped=$((skipped+1)); continue; }

  padded_month=$(printf '%02d' "$month")

  # locate source: try padded and unpadded
  src_dir=""
  for m in "$padded_month" "$month"; do
    cand="$VR/$guild/$channel/$year/$m"
    if [[ -f "$cand/$stem.ogg" ]]; then
      src_dir="$cand"; src_month="$m"; break
    fi
  done

  if [[ -z "$src_dir" ]]; then
    echo "MISS   $stem  (no file found under $VR/$guild/$channel/$year/{$month,$padded_month})"
    missing=$((missing+1))
    continue
  fi

  dst_dir="$VR/$guild/$channel/$year/$padded_month"
  dst="$dst_dir/$new_stem.ogg"
  src="$src_dir/$stem.ogg"

  if [[ -e "$dst" && "$dst" != "$src" ]]; then
    echo "CONFL  $stem -> $new_stem (target exists: $dst)"
    conflicts=$((conflicts+1))
    continue
  fi

  # -- plan the moves --
  ns_src=""; ns_dst=""
  ns_cand="$NVR/$guild/$channel/$year/$src_month/_no_silence_$stem.ogg"
  if [[ -f "$ns_cand" ]]; then
    ns_src="$ns_cand"
    ns_dst="$NVR/$guild/$channel/$year/$padded_month/_no_silence_$new_stem.ogg"
  fi

  wf_src=""; wf_dst=""
  wf_cand="$WD/$stem.dat"
  if [[ -f "$wf_cand" ]]; then
    wf_src="$wf_cand"
    wf_dst="$WD/$new_stem.dat"
  fi

  echo "MOVE   $src"
  echo "       -> $dst"
  [[ -n "$ns_src" ]] && echo "       + $ns_src -> $ns_dst"
  [[ -n "$wf_src" ]] && echo "       + $wf_src -> $wf_dst"
  echo "       + UPDATE audio_files SET file_name='$new_stem' WHERE file_name='$stem';"

  if [[ $APPLY -eq 1 ]]; then
    mkdir -p "$dst_dir"
    mv -n "$src" "$dst"
    if [[ -n "$ns_src" ]]; then
      mkdir -p "$(dirname "$ns_dst")"
      mv -n "$ns_src" "$ns_dst"
    fi
    if [[ -n "$wf_src" ]]; then
      mv -n "$wf_src" "$wf_dst"
    fi
    # DB update inside its own tx
    run_sql <<SQL
BEGIN;
UPDATE audio_files SET file_name = '$new_stem' WHERE file_name = '$stem';
COMMIT;
SQL
  fi

  moved=$((moved+1))
done < <(run_sql -c "$SQL_SELECT" | tr '|' '\t')

# Also pad any residual legacy month dirs that have no rows needing stem
# change (fresh writes after redeploy land in padded; consolidate old).
if [[ $APPLY -eq 1 ]]; then
  for root in "$VR" "$NVR"; do
    [[ -d "$root" ]] || continue
    while IFS= read -r -d '' d; do
      name=$(basename "$d"); [[ "$name" =~ ^[1-9]$ ]] || continue
      padded=$(printf '%02d' "$name")
      target="$(dirname "$d")/$padded"
      if [[ -d "$target" ]]; then
        shopt -s dotglob nullglob
        for c in "$d"/*; do mv -n "$c" "$target/"; done
        shopt -u dotglob nullglob
        rmdir "$d" 2>/dev/null || true
      else
        mv "$d" "$target"
      fi
    done < <(find "$root" -mindepth 4 -maxdepth 4 -type d -print0)
  done
fi

echo
echo "summary: total=$total moved=$moved skipped=$skipped missing=$missing conflicts=$conflicts"
[[ $APPLY -eq 0 ]] && echo "dry-run only. re-run with --apply to execute."

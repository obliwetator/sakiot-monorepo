#!/usr/bin/env bash

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '[deploy] %s\n' "$*"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

validate_tag() {
  [[ "$1" =~ ^v[0-9][0-9A-Za-z._-]*$ ]] || die "invalid release tag: $1"
}

validate_sha() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]] || die "invalid commit SHA: $1"
}

validate_tag_record() {
  local mode="$1"
  local record="$2"
  local tag="$3"
  local sha="$4"
  local recorded_sha

  if [[ "${mode}" == "release" && -e "${record}" ]]; then
    recorded_sha="$(cat "${record}")"
    if [[ "${recorded_sha}" == "${sha}" ]]; then
      die "release tag already deployed successfully: ${tag}"
    fi
    die "release tag was moved from ${recorded_sha} to ${sha}"
  fi

  if [[ "${mode}" == "rollback" ]]; then
    [[ -e "${record}" ]] || die "rollback tag was not previously deployed: ${tag}"
    recorded_sha="$(cat "${record}")"
    [[ "${recorded_sha}" == "${sha}" ]] || {
      die "rollback tag record does not match supplied SHA"
    }
  fi
}

run_systemctl() {
  if [[ "${SAKIOT_SYSTEMCTL_USE_SUDO:-1}" == "1" ]]; then
    sudo -n /usr/local/lib/sakiot-deploy/systemctl-wrapper "$@"
  else
    systemctl "$@"
  fi
}

atomic_symlink() {
  local target="$1"
  local link="$2"
  local temporary="${link}.new.$$"

  ln -s "${target}" "${temporary}"
  mv -Tf "${temporary}" "${link}"
}

json_array_from_lines() {
  jq -Rsc 'split("\n") | map(select(length > 0))'
}

# Bot unit name for a release id, or empty if the release has no bot artifact.
release_bot_unit() {
  local release_dir="$1"
  [[ -d "${release_dir}/fbi-agent" ]] || return 0
  printf 'sakiot-fbi-agent@%s.service' "$(basename "${release_dir}")"
}

# Remove old release directories under release_root, keeping the newest `keep`
# and never touching a release that is still in use (current web symlink target,
# current manifest, current bot, or any release whose bot unit is active such as
# a draining old instance).
prune_old_releases() {
  local release_root="$1"
  local current_root="$2"
  local state_dir="$3"
  local keep="$4"

  [[ -d "${release_root}" ]] || return 0
  [[ "${keep}" =~ ^[0-9]+$ ]] || {
    log "invalid SAKIOT_KEEP_RELEASES '${keep}'; skipping prune"
    return 0
  }

  local release_root_abs
  release_root_abs="$(cd "${release_root}" && pwd)"

  declare -A protected=()
  local target manifest unit unit_release

  target="$(readlink "${current_root}/web" 2>/dev/null || true)"
  [[ -n "${target}" ]] && protected["$(dirname "${target}")"]=1

  manifest="$(cat "${state_dir}/current.manifest" 2>/dev/null || true)"
  [[ -n "${manifest}" ]] && protected["$(dirname "${manifest}")"]=1

  unit="$(cat "${state_dir}/current-bot.unit" 2>/dev/null || true)"
  if [[ -n "${unit}" ]]; then
    unit_release="${unit#sakiot-fbi-agent@}"
    unit_release="${unit_release%.service}"
    protected["${release_root_abs}/${unit_release}"]=1
  fi

  # Sort newest -> oldest by the trailing release timestamp, falling back to
  # directory mtime when the id carries no timestamp.
  local -a ordered=()
  local dir stamp
  while IFS= read -r dir; do
    [[ -n "${dir}" ]] && ordered+=("${dir}")
  done < <(
    for dir in "${release_root_abs}"/*/; do
      [[ -d "${dir}" ]] || continue
      dir="${dir%/}"
      stamp="$(basename "${dir}" | grep -oE '[0-9]{8}T[0-9]{6}Z' | tail -n 1)"
      [[ -n "${stamp}" ]] || stamp="$(date -u -r "${dir}" +%Y%m%dT%H%M%SZ 2>/dev/null || echo 00000000T000000Z)"
      printf '%s\t%s\n' "${stamp}" "${dir}"
    done | sort -r | cut -f2-
  )

  local index=0
  for dir in "${ordered[@]}"; do
    index=$((index + 1))
    [[ "${index}" -le "${keep}" ]] && continue
    [[ -n "${protected[${dir}]:-}" ]] && continue

    unit="$(release_bot_unit "${dir}")"
    if [[ -n "${unit}" ]] && run_systemctl is-active --quiet "${unit}"; then
      log "keeping in-use release ${dir}"
      continue
    fi

    # Path safety: only ever remove a direct child of release_root.
    case "${dir}" in
      "${release_root_abs}"/*) ;;
      *) log "refusing to prune path outside release root: ${dir}"; continue ;;
    esac

    if [[ -n "${unit}" ]]; then
      run_systemctl disable "${unit}" >/dev/null 2>&1 || true
    fi
    log "pruning old release ${dir}"
    rm -rf "${dir}"
  done
}

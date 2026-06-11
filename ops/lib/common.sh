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
  # Strict semver. Suffixes/typos (v1.23, v1.2.3-rc1) are rejected so only
  # intentional production releases deploy.
  [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid release tag: $1"
}

validate_sha() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]] || die "invalid commit SHA: $1"
}

database_name_from_url() {
  local url="$1"
  local without_query="${url%%\?*}"
  local database="${without_query##*/}"

  [[ "${url}" == postgres://* || "${url}" == postgresql://* ]] \
    || die "database URL must use postgres:// or postgresql://"
  [[ -n "${database}" && "${database}" != "${without_query}" ]] \
    || die "database URL must include a database name"
  printf '%s\n' "${database}"
}

validate_test_database_url() {
  local runtime_url="$1"
  local test_url="$2"
  local runtime_database
  local test_database

  [[ -n "${test_url}" ]] || die "SAKIOT_TEST_DATABASE_URL must be set"
  runtime_database="$(database_name_from_url "${runtime_url}")"
  test_database="$(database_name_from_url "${test_url}")"
  [[ "${test_database}" == *_test ]] \
    || die "SAKIOT_TEST_DATABASE_URL database must end in _test"
  [[ "${test_database}" != "${runtime_database}" ]] \
    || die "test and runtime database names must differ"
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
  local bot_unit_prefix="$2"
  [[ -d "${release_dir}/fbi-agent" ]] || return 0
  printf '%s%s.service' "${bot_unit_prefix}" "$(basename "${release_dir}")"
}

# Print release dirs under release_root newest -> oldest, ordered by the trailing
# release timestamp in the id, falling back to directory mtime when absent.
releases_newest_first() {
  local release_root_abs="$1"
  local dir stamp
  for dir in "${release_root_abs}"/*/; do
    [[ -d "${dir}" ]] || continue
    dir="${dir%/}"
    stamp="$(basename "${dir}" | grep -oE '[0-9]{8}T[0-9]{6}Z' | tail -n 1)"
    [[ -n "${stamp}" ]] || stamp="$(date -u -r "${dir}" +%Y%m%dT%H%M%SZ 2>/dev/null || echo 00000000T000000Z)"
    printf '%s\t%s\n' "${stamp}" "${dir}"
  done | sort -r | cut -f2-
}

# Find a prior release dir whose manifest SHA matches target_sha and that holds a
# usable artifact for `component` (bot|web|frontend). Prints the newest match, or
# nothing. exclude_dir (the release being built now) is skipped.
reusable_artifact() {
  local release_root="$1"
  local target_sha="$2"
  local component="$3"
  local exclude_dir="$4"

  [[ -d "${release_root}" ]] || return 0
  local release_root_abs
  release_root_abs="$(cd "${release_root}" && pwd)"

  local dir manifest_sha
  while IFS= read -r dir; do
    [[ -n "${dir}" ]] || continue
    [[ "${dir}" == "${exclude_dir}" ]] && continue
    [[ -f "${dir}/manifest.json" ]] || continue
    manifest_sha="$(jq -r '.sha // empty' "${dir}/manifest.json" 2>/dev/null || true)"
    [[ "${manifest_sha}" == "${target_sha}" ]] || continue

    case "${component}" in
      bot)      [[ -s "${dir}/fbi-agent/fbi_agent" ]] || continue ;;
      web)      [[ -s "${dir}/web/web_server" ]] || continue ;;
      frontend) [[ -d "${dir}/frontend/dist" ]] && [[ -n "$(ls -A "${dir}/frontend/dist" 2>/dev/null)" ]] || continue ;;
      *) return 0 ;;
    esac

    printf '%s\n' "${dir}"
    return 0
  done < <(releases_newest_first "${release_root_abs}")
  return 0
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
  local bot_unit_prefix="$5"

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
    unit_release="${unit#"${bot_unit_prefix}"}"
    unit_release="${unit_release%.service}"
    protected["${release_root_abs}/${unit_release}"]=1
  fi

  # Newest -> oldest so the first `keep` are retained.
  local -a ordered=()
  local dir
  while IFS= read -r dir; do
    [[ -n "${dir}" ]] && ordered+=("${dir}")
  done < <(releases_newest_first "${release_root_abs}")

  local index=0
  for dir in "${ordered[@]}"; do
    index=$((index + 1))
    [[ "${index}" -le "${keep}" ]] && continue
    [[ -n "${protected[${dir}]:-}" ]] && continue

    unit="$(release_bot_unit "${dir}" "${bot_unit_prefix}")"
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

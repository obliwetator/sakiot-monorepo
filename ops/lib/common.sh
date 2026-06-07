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

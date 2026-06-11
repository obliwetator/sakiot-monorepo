#!/usr/bin/env bash
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/common.sh
source "${test_dir}/../lib/common.sh"

# Run prune without sudo so run_systemctl calls the shimmed `systemctl` on PATH.
export SAKIOT_SYSTEMCTL_USE_SUDO=0

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

# Build a fake deployment tree: 7 releases with increasing timestamps, each with
# an fbi-agent/ subdir; current points at the newest.
make_tree() {
  local root="$1"
  local prefix="$2"
  local release_root="${root}/releases"
  local current_root="${root}/current"
  local state_dir="${root}/state"
  mkdir -p "${release_root}" "${current_root}" "${state_dir}"

  local i id
  for i in 1 2 3 4 5 6 7; do
    id="v1.0.${i}-abc123def456-2026010${i}T120000Z"
    mkdir -p "${release_root}/${id}/fbi-agent" "${release_root}/${id}/web"
    printf 'binary\n' >"${release_root}/${id}/web/web_server"
  done

  local newest="v1.0.7-abc123def456-20260107T120000Z"
  ln -s "${release_root}/${newest}/web" "${current_root}/web"
  printf '%s\n' "${release_root}/${newest}/manifest.json" >"${state_dir}/current.manifest"
  printf '%s%s.service\n' "${prefix}" "${newest}" >"${state_dir}/current-bot.unit"
}

# --- Case 1: nothing active, keep=5 -> two oldest pruned --------------------
run_case_no_active() {
  local root shimdir
  root="$(mktemp -d)"
  shimdir="$(mktemp -d)"
  trap 'rm -rf "${root}" "${shimdir}"' RETURN

  cat >"${shimdir}/systemctl" <<'SH'
#!/usr/bin/env bash
# is-active always fails (nothing draining); everything else succeeds.
case "$1" in
  is-active) exit 3 ;;
  *) exit 0 ;;
esac
SH
  chmod +x "${shimdir}/systemctl"

  make_tree "${root}" sakiot-fbi-agent@
  PATH="${shimdir}:${PATH}" \
    prune_old_releases "${root}/releases" "${root}/current" "${root}/state" 5 \
      sakiot-fbi-agent@

  local remaining
  remaining="$(find "${root}/releases" -maxdepth 1 -mindepth 1 -type d | wc -l)"
  [[ "${remaining}" -eq 5 ]] || fail "expected 5 releases kept, got ${remaining}"

  [[ -d "${root}/releases/v1.0.7-abc123def456-20260107T120000Z" ]] \
    || fail "newest release was pruned"
  [[ ! -d "${root}/releases/v1.0.1-abc123def456-20260101T120000Z" ]] \
    || fail "oldest release v1.0.1 should have been pruned"
  [[ ! -d "${root}/releases/v1.0.2-abc123def456-20260102T120000Z" ]] \
    || fail "release v1.0.2 should have been pruned"
}

# --- Case 2: an OLD bot unit is active (draining) -> that dir is kept --------
run_case_draining() {
  local root shimdir
  root="$(mktemp -d)"
  shimdir="$(mktemp -d)"
  trap 'rm -rf "${root}" "${shimdir}"' RETURN

  # Simulate the oldest release still draining: is-active succeeds only for it.
  cat >"${shimdir}/systemctl" <<'SH'
#!/usr/bin/env bash
if [[ "$1" == "is-active" ]]; then
  for arg in "$@"; do
    [[ "${arg}" == *"v1.0.1-"* ]] && exit 0
  done
  exit 3
fi
exit 0
SH
  chmod +x "${shimdir}/systemctl"

  make_tree "${root}" sakiot-fbi-agent@
  PATH="${shimdir}:${PATH}" \
    prune_old_releases "${root}/releases" "${root}/current" "${root}/state" 5 \
      sakiot-fbi-agent@

  [[ -d "${root}/releases/v1.0.1-abc123def456-20260101T120000Z" ]] \
    || fail "draining release v1.0.1 must be kept despite being beyond keep=5"
  # v1.0.2 has no active unit and is still beyond keep -> pruned.
  [[ ! -d "${root}/releases/v1.0.2-abc123def456-20260102T120000Z" ]] \
    || fail "non-draining old release v1.0.2 should have been pruned"
}

# --- Case 3: staging prefix -> staging units queried, draining one kept ------
run_case_staging_prefix() {
  local root shimdir
  root="$(mktemp -d)"
  shimdir="$(mktemp -d)"
  trap 'rm -rf "${root}" "${shimdir}"' RETURN

  # is-active succeeds only for the staging unit of v1.0.1; a query using the
  # production prefix would not match and v1.0.1 would be wrongly pruned.
  cat >"${shimdir}/systemctl" <<'SH'
#!/usr/bin/env bash
if [[ "$1" == "is-active" ]]; then
  for arg in "$@"; do
    [[ "${arg}" == sakiot-staging-fbi-agent@*"v1.0.1-"* ]] && exit 0
  done
  exit 3
fi
exit 0
SH
  chmod +x "${shimdir}/systemctl"

  make_tree "${root}" sakiot-staging-fbi-agent@
  PATH="${shimdir}:${PATH}" \
    prune_old_releases "${root}/releases" "${root}/current" "${root}/state" 5 \
      sakiot-staging-fbi-agent@

  [[ -d "${root}/releases/v1.0.1-abc123def456-20260101T120000Z" ]] \
    || fail "draining staging release v1.0.1 must be kept"
  [[ ! -d "${root}/releases/v1.0.2-abc123def456-20260102T120000Z" ]] \
    || fail "non-draining old staging release v1.0.2 should have been pruned"
}

run_case_no_active
run_case_draining
run_case_staging_prefix
echo "release_gc_test ok"

#!/usr/bin/env bash
set -Eeuo pipefail
umask 027

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
source "${script_dir}/lib/common.sh"
# shellcheck source=lib/components.sh
source "${script_dir}/lib/components.sh"

mode="${1:-}"
schema_override=""
case "${mode}" in
  release|rollback)
    target="production"
    tag="${2:-}"
    sha="${3:-}"
    schema_override="${4:-}"
    validate_tag "${tag}"
    validate_sha "${sha}"
    if [[ -n "${schema_override}" && "${schema_override}" != "--allow-schema-mismatch" ]]; then
      die "invalid rollback option"
    fi
    ;;
  stage)
    target="staging"
    tag="main"
    sha="${2:-}"
    validate_sha "${sha}"
    ;;
  *)
    die "invalid deployment mode"
    ;;
esac

default_env_file="/etc/sakiot/production.env"
[[ "${target}" == "staging" ]] && default_env_file="/etc/sakiot/staging.env"
env_file="${SAKIOT_ENV_FILE:-${default_env_file}}"
if [[ -f "${env_file}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${env_file}"
  set +a
fi

web_unit="${SAKIOT_WEB_UNIT:-sakiot-web.service}"
bot_unit_prefix="${SAKIOT_BOT_UNIT_PREFIX:-sakiot-fbi-agent@}"

repository_url="${SAKIOT_REPOSITORY_URL:?set SAKIOT_REPOSITORY_URL}"
[[ "${repository_url}" == https://* ]] || die "SAKIOT_REPOSITORY_URL must use HTTPS"

data_dir="${SAKIOT_DATA_DIR:-/var/lib/sakiot/data}"
state_dir="${SAKIOT_DEPLOY_STATE_DIR:-/var/lib/sakiot/deploy}"
release_root="${SAKIOT_RELEASE_ROOT:-/srv/sakiot/releases}"
current_root="${SAKIOT_CURRENT_ROOT:-/srv/sakiot/current}"
cache_dir="${SAKIOT_CACHE_DIR:-/var/cache/sakiot}"
source_repo="${cache_dir}/repository"
worktree_root="${cache_dir}/worktrees"
frontend_root="${SAKIOT_FRONTEND_ROOT:-/var/www/patrykstyla.com}"
web_health_url="${SAKIOT_WEB_HEALTH_URL:-http://127.0.0.1:8900/healthz}"
web_registry_url="${SAKIOT_WEB_REGISTRY_URL:-http://127.0.0.1:8900/internal/fbi-agent/grpc-endpoints}"
legacy_bot_unit="${SAKIOT_LEGACY_BOT_UNIT:-}"
legacy_bot_grpc="${SAKIOT_LEGACY_BOT_GRPC:-}"
legacy_web_enabled="${SAKIOT_LEGACY_WEB_ENABLED:-0}"

for command in git flock jq cargo curl rsync install; do
  require_command "${command}"
done
if [[ "${SAKIOT_SYSTEMCTL_USE_SUDO:-1}" == "1" ]]; then
  require_command sudo
  [[ -x /usr/local/lib/sakiot-deploy/systemctl-wrapper ]] \
    || die "systemctl wrapper is not installed"
else
  require_command systemctl
fi

install -d -m 0750 "${state_dir}" "${state_dir}/tags" "${release_root}" \
  "${current_root}" "${cache_dir}" "${worktree_root}"
install -d -m 0755 "${data_dir}" "${data_dir}/voice_recordings" \
  "${data_dir}/no_silence_voice_recordings" "${data_dir}/waveform_data" \
  "${data_dir}/clips"

exec 9>"${state_dir}/deploy.lock"
flock 9

if [[ ! -d "${source_repo}/.git" ]]; then
  log "creating deployment repository cache"
  git clone --no-checkout "${repository_url}" "${source_repo}"
fi

git -C "${source_repo}" remote set-url origin "${repository_url}"
if [[ "${target}" == "production" ]]; then
  git -C "${source_repo}" fetch --prune origin \
    '+refs/heads/*:refs/remotes/origin/*' \
    "+refs/tags/${tag}:refs/tags/${tag}"

  resolved_sha="$(git -C "${source_repo}" rev-list -n 1 "refs/tags/${tag}")"
  [[ "${resolved_sha}" == "${sha}" ]] || {
    die "tag ${tag} resolves to ${resolved_sha}, not supplied SHA ${sha}"
  }
else
  git -C "${source_repo}" fetch --prune origin \
    '+refs/heads/*:refs/remotes/origin/*'
fi
git -C "${source_repo}" cat-file -e "${sha}^{commit}"

if [[ "${target}" == "production" ]]; then
  tag_record="${state_dir}/tags/${tag}"
  validate_tag_record "${mode}" "${tag_record}" "${tag}" "${sha}"
fi

previous_sha="$(cat "${state_dir}/current.sha" 2>/dev/null || true)"
previous_tag="$(cat "${state_dir}/current.tag" 2>/dev/null || true)"

if [[ "${mode}" == "rollback" && -n "${previous_sha}" && "${schema_override}" != "--allow-schema-mismatch" ]]; then
  mapfile -t migration_changes < <(
    git -C "${source_repo}" diff --name-only "${sha}" "${previous_sha}" -- sakiot-db/migrations
  )
  if [[ "${#migration_changes[@]}" -gt 0 ]]; then
    die "rollback crosses migration changes; use explicit schema compatibility override"
  fi
fi

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ "${mode}" == "rollback" ]]; then
  release_id="${tag}-${sha:0:12}-rollback-${timestamp}"
elif [[ "${mode}" == "stage" ]]; then
  release_id="staging-${sha:0:12}-${timestamp}"
else
  release_id="${tag}-${sha:0:12}-${timestamp}"
fi
artifact_dir="${release_root}/${release_id}"
worktree="${worktree_root}/${release_id}"

cleanup_worktree() {
  git -C "${source_repo}" worktree remove --force "${worktree}" >/dev/null 2>&1 || true
}
trap cleanup_worktree EXIT
cleanup_worktree
git -C "${source_repo}" worktree add --detach "${worktree}" "${sha}"

if [[ "${mode}" == "rollback" || -z "${previous_sha}" ]]; then
  mapfile -t components < <(all_components)
  if [[ "${mode}" == "rollback" ]]; then
    components=(bot web frontend)
  fi
else
  mapfile -t changed_paths < <(
    git -C "${source_repo}" diff --name-only "${previous_sha}" "${sha}"
  )
  mapfile -t components < <(components_for_paths "${changed_paths[@]}")
fi

if [[ "${#components[@]}" -eq 0 ]]; then
  log "documentation-only release; no application components selected"
fi

if [[ -e "${artifact_dir}" ]]; then
  die "release directory already exists: ${artifact_dir}"
fi
install -d -m 0755 "${artifact_dir}"

# On rollback, reuse the binaries/dist already built for this exact SHA instead
# of recompiling. Resolved per component; empty means build from source.
reuse_bot=""
reuse_web=""
reuse_frontend=""
if [[ "${mode}" == "rollback" && "${SAKIOT_ROLLBACK_FORCE_REBUILD:-0}" != "1" ]]; then
  component_selected bot "${components[@]}" \
    && reuse_bot="$(reusable_artifact "${release_root}" "${sha}" bot "${artifact_dir}")"
  component_selected web "${components[@]}" \
    && reuse_web="$(reusable_artifact "${release_root}" "${sha}" web "${artifact_dir}")"
  component_selected frontend "${components[@]}" \
    && reuse_frontend="$(reusable_artifact "${release_root}" "${sha}" frontend "${artifact_dir}")"
fi

# Only test/compile Rust when a Rust component is actually built from source.
# Reused binaries were already tested at their original deploy.
build_rust=0
component_selected bot "${components[@]}" && [[ -z "${reuse_bot}" ]] && build_rust=1
component_selected web "${components[@]}" && [[ -z "${reuse_web}" ]] && build_rust=1
if [[ "${build_rust}" == "1" ]]; then
  require_command protoc
  validate_test_database_url "${DATABASE_URL:?set DATABASE_URL}" \
    "${SAKIOT_TEST_DATABASE_URL:-}"
  log "testing Rust workspace"
  (
    cd "${worktree}"
    test_data_dir="$(mktemp -d "${cache_dir}/test-data.XXXXXX")"
    trap 'rm -rf "${test_data_dir}"' EXIT
    DATABASE_URL="${SAKIOT_TEST_DATABASE_URL}" \
      SAKIOT_DATA_DIR="${test_data_dir}" \
      SQLX_OFFLINE=true CARGO_TARGET_DIR="${cache_dir}/cargo-target" \
      cargo test --workspace --locked
  )
fi

if component_selected bot "${components[@]}"; then
  install -d -m 0755 "${artifact_dir}/fbi-agent"
  if [[ -n "${reuse_bot}" ]]; then
    log "reusing FBI Agent artifact from ${reuse_bot}"
    install -m 0755 "${reuse_bot}/fbi-agent/fbi_agent" \
      "${artifact_dir}/fbi-agent/fbi_agent"
  else
    log "building FBI Agent"
    (
      cd "${worktree}"
      SQLX_OFFLINE=true CARGO_TARGET_DIR="${cache_dir}/cargo-target" \
        cargo build --release --locked --package fbi_agent
    )
    install -m 0755 "${cache_dir}/cargo-target/release/fbi_agent" \
      "${artifact_dir}/fbi-agent/fbi_agent"
  fi
fi

if component_selected web "${components[@]}"; then
  install -d -m 0755 "${artifact_dir}/web"
  if [[ -n "${reuse_web}" ]]; then
    log "reusing web server artifact from ${reuse_web}"
    install -m 0755 "${reuse_web}/web/web_server" \
      "${artifact_dir}/web/web_server"
  else
    log "building web server"
    (
      cd "${worktree}"
      SQLX_OFFLINE=true CARGO_TARGET_DIR="${cache_dir}/cargo-target" \
        cargo build --release --locked --package web_server
    )
    install -m 0755 "${cache_dir}/cargo-target/release/web_server" \
      "${artifact_dir}/web/web_server"
  fi
fi

if component_selected frontend "${components[@]}"; then
  install -d -m 0755 "${artifact_dir}/frontend"
  if [[ -n "${reuse_frontend}" ]]; then
    log "reusing frontend artifact from ${reuse_frontend}"
    cp -a "${reuse_frontend}/frontend/dist" "${artifact_dir}/frontend/dist"
  else
    require_command bun
    log "testing and building frontend"
    (
      cd "${worktree}/sakiot_stage"
      bun install --frozen-lockfile
      bun run test
      SAKIOT_RELEASE_TAG="${tag}" \
        SAKIOT_COMMIT_SHA="${sha}" \
        SAKIOT_BUNDLE_VERSION="${release_id}" \
        bun run build
    )
    cp -a "${worktree}/sakiot_stage/dist" "${artifact_dir}/frontend/dist"
  fi
fi

if [[ -x "${worktree}/ops/tests/run.sh" ]]; then
  log "testing deployment scripts"
  "${worktree}/ops/tests/run.sh"
fi

migration_head="$(
  find "${worktree}/sakiot-db/migrations" -maxdepth 1 -type f -printf '%f\n' \
    | sort | tail -n 1
)"
migrations_ran=false
if component_selected database "${components[@]}"; then
  require_command sqlx
  log "checking migration state"
  sqlx migrate info --source "${worktree}/sakiot-db/migrations"
  if [[ "${SAKIOT_SKIP_DB_BACKUP:-0}" == "1" ]]; then
    log "SAKIOT_SKIP_DB_BACKUP=1: applying migrations without a pre-migrate backup"
    sqlx migrate run --source "${worktree}/sakiot-db/migrations"
  else
    require_command pg_dump
    require_command age
    log "backing up database and applying pending migrations"
    SAKIOT_ENV_FILE="${env_file}" \
      "${worktree}/sakiot-db/ops/backup/pre-migrate-backup.sh"
  fi
  migrations_ran=true
fi

proto_dir="${worktree}/sakiot-proto/proto"
bot_recovery_required=0
new_bot_started=0
bot_handoff_pending=0
old_bot_disabled=0
new_bot_unit=""
old_bot_unit=""
old_bot_grpc=""
old_bot_is_legacy=0
previous_bot_unit=""
previous_bot_grpc=""

publish_bot_registry() {
  local active="$1"
  local draining_json="$2"
  local registry_body
  local -a curl_args=(-fsS -H 'Content-Type: application/json')

  registry_body="$(
    jq -cn --arg active "${active}" --argjson draining "${draining_json}" \
      '{active:$active,draining:$draining}'
  )"
  curl_args+=(--connect-timeout 2 --max-time 5)
  if [[ -n "${FBI_AGENT_REGISTRY_SECRET:-}" ]]; then
    curl_args+=(-H "X-FBI-Agent-Registry-Secret: ${FBI_AGENT_REGISTRY_SECRET}")
  fi
  curl "${curl_args[@]}" -d "${registry_body}" "${web_registry_url}" >/dev/null
}

cancel_old_bot_drain() {
  if [[ "${bot_recovery_required}" != "1" || -z "${old_bot_grpc}" ]]; then
    return
  fi
  log "new FBI Agent failed readiness; cancelling old instance drain"
  if grpcurl -max-time 3 -plaintext -import-path "${proto_dir}" -proto fbi_agent.proto \
    -d "{\"reason\":\"release ${release_id} failed readiness\"}" \
    "${old_bot_grpc}" fbi_agent.Admin/CancelDrain >/dev/null; then
    return
  fi
  if [[ "${old_bot_is_legacy}" == "1" ]]; then
    log "legacy FBI Agent lacks CancelDrain; restarting it to clear drain state"
    run_systemctl legacy-bot-restart "${old_bot_unit}" || \
      log "legacy FBI Agent restart unavailable; manual restart required"
  fi
  return 0
}
recover_bot_on_error() {
  local status=$?

  trap - ERR
  set +e
  if [[ "${new_bot_started}" == "1" && -n "${new_bot_unit}" ]]; then
    run_systemctl stop "${new_bot_unit}" || true
    run_systemctl disable "${new_bot_unit}" || true
    printf '%s\n' "${previous_bot_unit}" >"${state_dir}/current-bot.unit"
    printf '%s\n' "${previous_bot_grpc}" >"${state_dir}/current-bot.grpc"
  fi
  if [[ "${old_bot_disabled}" == "1" && -n "${old_bot_unit}" ]]; then
    if [[ "${old_bot_is_legacy}" == "1" ]]; then
      run_systemctl legacy-bot-enable "${old_bot_unit}" || true
    else
      run_systemctl enable "${old_bot_unit}" || true
    fi
  fi
  cancel_old_bot_drain
  if [[ -n "${old_bot_grpc}" ]]; then
    publish_bot_registry "${old_bot_grpc}" '[]' || \
      log "failed to restore old FBI Agent in web registry"
  fi
  exit "${status}"
}
trap recover_bot_on_error ERR

fail_deployment() {
  printf 'error: %s\n' "$*" >&2
  return 1
}

if component_selected bot "${components[@]}"; then
  require_command grpcurl
  require_command python3

  old_bot_unit="$(cat "${state_dir}/current-bot.unit" 2>/dev/null || true)"
  old_bot_grpc="$(cat "${state_dir}/current-bot.grpc" 2>/dev/null || true)"
  previous_bot_unit="${old_bot_unit}"
  previous_bot_grpc="${old_bot_grpc}"
  new_bot_unit="${bot_unit_prefix}${release_id}.service"
  new_bot_grpc="127.0.0.1:$(
    python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
  )"

  cat >"${artifact_dir}/fbi-agent/service.env" <<EOF
BOT_ROLE=active
BOT_INSTANCE_ID=$(hostname)-${release_id}
RELEASE_ID=${release_id}
GRPC_ADDR=${new_bot_grpc}
DRAIN_TIMEOUT_SECONDS=0
SAKIOT_DATA_DIR=${data_dir}
EOF
  chmod 0640 "${artifact_dir}/fbi-agent/service.env"

  old_bot_active=0
  if [[ -n "${old_bot_unit}" ]] && run_systemctl is-active --quiet "${old_bot_unit}"; then
    old_bot_active=1
  elif [[ -z "${old_bot_unit}" && -n "${legacy_bot_unit}" && -n "${legacy_bot_grpc}" ]] \
    && run_systemctl legacy-bot-is-active "${legacy_bot_unit}"; then
    old_bot_unit="${legacy_bot_unit}"
    old_bot_grpc="${legacy_bot_grpc}"
    old_bot_is_legacy=1
    old_bot_active=1
    log "adopting legacy FBI Agent ${old_bot_unit} for first-release handoff"
  fi

  if [[ "${old_bot_active}" == "1" ]]; then
    [[ -n "${old_bot_grpc}" ]] || die "current FBI Agent is missing its gRPC address"
    log "draining ${old_bot_unit}"
    grpcurl -max-time 3 -plaintext -import-path "${proto_dir}" -proto fbi_agent.proto \
      -d "{\"reason\":\"deploy ${release_id}\"}" \
      "${old_bot_grpc}" fbi_agent.Admin/StartDrain >/dev/null
    bot_recovery_required=1
  else
    old_bot_unit=""
    old_bot_grpc=""
  fi

  log "starting ${new_bot_unit}"
  if ! run_systemctl start "${new_bot_unit}"; then
    run_systemctl stop "${new_bot_unit}" || true
    cancel_old_bot_drain
    exit 1
  fi
  new_bot_started=1

  bot_ready=0
  for _ in $(seq 1 30); do
    if grpcurl -max-time 3 -plaintext -import-path "${proto_dir}" -proto fbi_agent.proto \
      "${new_bot_grpc}" fbi_agent.Admin/GetDrainStatus >/dev/null 2>&1; then
      bot_ready=1
      break
    fi
    sleep 1
  done
  if [[ "${bot_ready}" != "1" ]]; then
    run_systemctl stop "${new_bot_unit}" || true
    new_bot_started=0
    cancel_old_bot_drain
    die "new FBI Agent failed readiness"
  fi

  run_systemctl enable "${new_bot_unit}" >/dev/null
  printf '%s\n' "${new_bot_unit}" >"${state_dir}/current-bot.unit"
  printf '%s\n' "${new_bot_grpc}" >"${state_dir}/current-bot.grpc"

  draining_json='[]'
  if [[ -n "${old_bot_grpc}" ]]; then
    draining_json="$(jq -cn --arg address "${old_bot_grpc}" '[$address]')"
  fi
  publish_bot_registry "${new_bot_grpc}" "${draining_json}" \
    || log "web server registry unavailable; web release env will use new endpoint"

  if [[ -n "${old_bot_grpc}" ]]; then
    bot_handoff_pending=1
  fi
fi

if component_selected web "${components[@]}"; then
  active_bot_grpc="$(cat "${state_dir}/current-bot.grpc" 2>/dev/null || true)"
  cat >"${artifact_dir}/web/service.env" <<EOF
RELEASE_ID=${release_id}
SAKIOT_DATA_DIR=${data_dir}
GRPC_ADDRESS=http://${active_bot_grpc:-127.0.0.1:50052}
EOF
  chmod 0640 "${artifact_dir}/web/service.env"

  previous_web_target="$(readlink "${current_root}/web" 2>/dev/null || true)"
  legacy_web_stopped=0
  restore_previous_web() {
    if [[ -n "${previous_web_target}" ]]; then
      atomic_symlink "${previous_web_target}" "${current_root}/web"
      run_systemctl restart "${web_unit}" || true
    elif [[ "${legacy_web_stopped}" == "1" ]]; then
      run_systemctl legacy-web-start-enable || true
    fi
  }
  atomic_symlink "${artifact_dir}/web" "${current_root}/web"
  if [[ -z "${previous_web_target}" && "${legacy_web_enabled}" == "1" ]] \
    && run_systemctl legacy-web-is-active; then
    log "stopping legacy web server for first-release handoff"
    if ! run_systemctl legacy-web-stop-disable; then
      run_systemctl legacy-web-start-enable || true
      fail_deployment "failed to stop legacy web server"
    fi
    legacy_web_stopped=1
  fi
  log "restarting web server"
  if ! run_systemctl restart "${web_unit}"; then
    restore_previous_web
    fail_deployment "web server restart failed"
  fi
  if ! run_systemctl enable-web "${web_unit}" >/dev/null; then
    log "web enable action unavailable; install updated production controls"
  fi

  web_ready=0
  for _ in $(seq 1 30); do
    if curl -fsS --connect-timeout 2 --max-time 5 "${web_health_url}" \
      | jq -e --arg release "${release_id}" \
        '.status == "ok" and .database == "ready" and .release_id == $release' \
        >/dev/null; then
      web_ready=1
      break
    fi
    sleep 1
  done
  if [[ "${web_ready}" != "1" ]]; then
    run_systemctl stop "${web_unit}" || true
    restore_previous_web
    fail_deployment "web server failed readiness; previous release restored"
  fi
fi

if component_selected frontend "${components[@]}"; then
  install -d -m 0755 "${frontend_root}"
  log "publishing frontend assets, HTML, then version metadata"
  SAKIOT_FRONTEND_ROOT="${frontend_root}" \
    SAKIOT_FRONTEND_DIST="${artifact_dir}/frontend/dist" \
    "${worktree}/sakiot_stage/scripts/deploy.sh"
fi

if [[ "${bot_handoff_pending}" == "1" ]]; then
  if [[ "${old_bot_is_legacy}" == "1" ]]; then
    run_systemctl legacy-bot-disable "${old_bot_unit}" >/dev/null
  else
    run_systemctl disable "${old_bot_unit}" >/dev/null
  fi
  old_bot_disabled=1
  grpcurl -max-time 3 -plaintext -import-path "${proto_dir}" -proto fbi_agent.proto \
    -d "{\"reason\":\"release ${release_id} is fully ready\"}" \
    "${old_bot_grpc}" fbi_agent.Admin/ShutdownWhenEmpty >/dev/null
fi

if component_selected bot "${components[@]}"; then
  for stale_bot_dir in "${release_root}"/*/fbi-agent; do
    [[ -d "${stale_bot_dir}" ]] || continue
    stale_release_id="$(basename "$(dirname "${stale_bot_dir}")")"
    stale_bot_unit="${bot_unit_prefix}${stale_release_id}.service"
    [[ "${stale_bot_unit}" == "${new_bot_unit}" ]] && continue
    run_systemctl disable "${stale_bot_unit}" >/dev/null || true
  done
fi
bot_recovery_required=0
new_bot_started=0
old_bot_disabled=0

components_json="$(printf '%s\n' "${components[@]}" | json_array_from_lines)"
changed_paths_json="$(
  if [[ -n "${changed_paths+x}" ]]; then
    printf '%s\n' "${changed_paths[@]}" | json_array_from_lines
  else
    printf '[]'
  fi
)"
reused_json="$(
  jq -cn \
    --argjson bot "$([[ -n "${reuse_bot}" ]] && echo true || echo false)" \
    --argjson web "$([[ -n "${reuse_web}" ]] && echo true || echo false)" \
    --argjson frontend "$([[ -n "${reuse_frontend}" ]] && echo true || echo false)" \
    '{bot:$bot,web:$web,frontend:$frontend}'
)"
manifest="${artifact_dir}/manifest.json"
jq -n \
  --arg target "${target}" \
  --arg mode "${mode}" \
  --arg tag "${tag}" \
  --arg sha "${sha}" \
  --arg previous_tag "${previous_tag}" \
  --arg previous_sha "${previous_sha}" \
  --arg release_id "${release_id}" \
  --arg deployed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg migration_head "${migration_head}" \
  --argjson components "${components_json}" \
  --argjson changed_paths "${changed_paths_json}" \
  --argjson migrations_ran "${migrations_ran}" \
  --argjson reused "${reused_json}" \
  '{
    target:$target,
    mode:$mode,
    tag:$tag,
    sha:$sha,
    previous_tag:$previous_tag,
    previous_sha:$previous_sha,
    release_id:$release_id,
    components:$components,
    changed_paths:$changed_paths,
    database:{migrations_ran:$migrations_ran,migration_head:$migration_head},
    reused:$reused,
    deployed_at:$deployed_at
  }' >"${manifest}"

printf '%s\n' "${sha}" >"${state_dir}/current.sha.new"
mv "${state_dir}/current.sha.new" "${state_dir}/current.sha"
printf '%s\n' "${tag}" >"${state_dir}/current.tag.new"
mv "${state_dir}/current.tag.new" "${state_dir}/current.tag"
printf '%s\n' "${manifest}" >"${state_dir}/current.manifest.new"
mv "${state_dir}/current.manifest.new" "${state_dir}/current.manifest"
if [[ "${mode}" == "release" ]]; then
  printf '%s\n' "${sha}" >"${tag_record}"
fi

keep_releases="${SAKIOT_KEEP_RELEASES:-5}"
prune_old_releases "${release_root}" "${current_root}" "${state_dir}" \
  "${keep_releases}" "${bot_unit_prefix}" \
  || log "release pruning encountered an error; continuing"

log "${mode} complete: ${release_id}"
log "newest ${keep_releases} releases retained for rollback"

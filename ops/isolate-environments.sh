#!/usr/bin/env bash
# Moves an existing host to per-environment runtime users and database roles:
# staging runs as sakiot-staging with the sakiot_staging role, preview slots as
# sakiot-preview with sakiot_slot. Neither can then read production's env
# file, backup key, data, processes, or database. Idempotent.
#
# Run as root from a checkout, right after ops/update-deploy-engine.sh
# installed the deploy engine from the same commit:
#
#   ops/isolate-environments.sh [--yes]
#       Staging, preview slots, and closing the production database to other
#       roles. Restarts staging and preview services; production keeps
#       running untouched.
#   ops/isolate-environments.sh --harden-production [--yes]
#       Once staging has run well on the hardened units: installs the
#       hardened production units and rotates the production database
#       password, which staging and preview processes could read until now.
#       Restarts the production web server and bot.
set -euo pipefail

phase=isolate
assume_yes=0
for argument in "$@"; do
  case "${argument}" in
    --harden-production) phase=production ;;
    --yes) assume_yes=1 ;;
    *) echo "usage: $0 [--harden-production] [--yes]" >&2; exit 2 ;;
  esac
done

log() { printf '\033[1;34m[isolate]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[isolate]\033[0m %s\n' "$*" >&2; exit 1; }

[[ "${EUID}" -eq 0 ]] || die "run as root"

ops_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${ops_dir}/.." && pwd)"
install_root=/usr/local/lib/sakiot-deploy
production_env=/etc/sakiot/production.env
staging_env=/etc/sakiot/staging.env
preview_env=/etc/sakiot/preview.env

confirm() {
  [[ "${assume_yes}" -eq 1 ]] && return 0
  local answer
  read -r -p "Continue? [y/N] " answer
  [[ "${answer}" == "y" || "${answer}" == "Y" ]] || die "aborted"
}

psql_admin() { sudo -u postgres psql -v ON_ERROR_STOP=1 -X -q "$@"; }

# Hands a database to its runtime role. The SQL goes in on stdin: the postgres
# user may not be able to read the checkout.
own_database() {
  local database="$1" role="$2"
  psql_admin -v owner="${role}" -d "${database}" -f - \
    <"${ops_dir}/sql/own-public-schema.sql" >/dev/null
}

# The role, password, or database of an env file's DATABASE_URL.
url_part() {
  local env_file="$1" part="$2"
  case "${part}" in
    user) sed -n 's|^DATABASE_URL=postgres\(ql\)\{0,1\}://\([^:@/]*\)[:@].*|\2|p' "${env_file}" ;;
    password) sed -n 's|^DATABASE_URL=postgres\(ql\)\{0,1\}://[^:@/]*:\([^@]*\)@.*|\2|p' "${env_file}" ;;
    database) sed -n 's|^DATABASE_URL=postgres\(ql\)\{0,1\}://[^/]*/\([^?]*\).*|\2|p' "${env_file}" ;;
  esac | head -n 1
}

identifier() {
  [[ "$1" =~ ^[a-z_][a-z0-9_]*$ ]] || die "unexpected identifier '$1'"
  printf '%s' "$1"
}

set_role_password() {
  local role="$1" password="$2"
  # Fed on stdin: `psql -c` would expose the password in argv.
  if psql_admin -tAc "SELECT 1 FROM pg_roles WHERE rolname = '${role}'" | grep -q 1; then
    printf "ALTER ROLE %s LOGIN PASSWORD '%s';\n" "${role}" "${password}" | psql_admin -f -
  else
    printf "CREATE ROLE %s LOGIN PASSWORD '%s';\n" "${role}" "${password}" | psql_admin -f -
    log "created role ${role}"
  fi
}

install_unit() {
  install -m 0644 "${ops_dir}/systemd/$1" "/etc/systemd/system/$1"
}

# Holds an instance's deploy lock, the engine's flock, until the script exits,
# so no deploy of that instance runs meanwhile. A missing lock file is created
# for the deploy user, which must be able to open it.
hold_deploy_lock() {
  local state_dir="$1" lock="$1/deploy.lock" fd
  [[ -d "${state_dir}" ]] || return 0
  [[ -e "${lock}" ]] || install -o sakiot -g sakiot -m 0640 /dev/null "${lock}"
  exec {fd}>>"${lock}"
  flock -n "${fd}" || die "a deploy is running (${lock}); wait for it to finish and re-run"
}

active_units() {
  systemctl list-units --type=service --state=active --no-legend --plain "$@" | awk '{print $1}'
}

# The units an instance runs: its web server and the bot the deploy state
# names. Also when stopped, so a re-run after a failure brings them back.
# Refuses while a previous bot is still draining: restarting it would bring
# back a second active bot.
instance_units() {
  local prefix="$1" state_dir="$2" current_bot bots
  current_bot="$(cat "${state_dir}/current-bot.unit" 2>/dev/null || true)"
  mapfile -t bots < <(active_units "${prefix}-fbi-agent@*.service")
  if [[ "${#bots[@]}" -gt 1 || ( "${#bots[@]}" -eq 1 && "${bots[0]}" != "${current_bot}" ) ]]; then
    die "${prefix} is draining a previous bot (${bots[*]}); wait for the drain to finish and re-run"
  fi
  {
    [[ -e "/srv/${prefix}/current/web" ]] && echo "${prefix}-web.service"
    printf '%s\n' "${current_bot}"
  } | grep -E "^${prefix}-(web|fbi-agent@[A-Za-z0-9._-]+)\.service$" || true
}

# Fails unless each unit runs as the expected user in the hardened sandbox.
failed=0
check_unit() {
  local unit="$1" expected_user="$2" pid user
  if ! systemctl is-active --quiet "${unit}"; then
    log "FAIL: ${unit} is not running (journalctl -u ${unit})"
    failed=1
    return
  fi
  pid="$(systemctl show -p MainPID --value "${unit}")"
  # ps truncates long user names; /proc/<pid> is owned by the process's user.
  user="$(stat -c %U "/proc/${pid}")"
  if [[ "${user}" != "${expected_user}" ]]; then
    log "FAIL: ${unit} runs as ${user}, expected ${expected_user}"
    failed=1
  elif [[ "$(systemctl show -p ProtectSystem --value "${unit}")" != strict ]]; then
    log "FAIL: ${unit} does not run in the hardened sandbox"
    failed=1
  else
    log "ok: ${unit} runs as ${user}, sandboxed"
  fi
}

# ---- production phase ---------------------------------------------------------
if [[ "${phase}" == production ]]; then
  production_role="$(identifier "$(url_part "${production_env}" user)")"
  hold_deploy_lock /var/lib/sakiot/deploy
  mapfile -t production_units < <(instance_units sakiot /var/lib/sakiot/deploy)
  log "plan:"
  log "  install the hardened sakiot-web.service and sakiot-fbi-agent@.service"
  log "  rotate the ${production_role} database password and update ${production_env} (backed up next to it)"
  log "  restart ${production_units[*]:-nothing}: the bot finalizes active recordings and rejoins, a short gap"
  confirm
  install_unit sakiot-web.service
  install_unit sakiot-fbi-agent@.service
  systemctl daemon-reload
  # The env file first: the running services keep their connections until
  # the restart right after.
  password="$(openssl rand -hex 24)"
  "${ops_dir}/rewrite-db-role.sh" "${production_env}" "${production_role}" "${password}"
  set_role_password "${production_role}" "${password}"
  if [[ "${#production_units[@]}" -gt 0 ]]; then
    systemctl restart "${production_units[@]}"
  fi
  sleep 5
  for unit in "${production_units[@]}"; do
    check_unit "${unit}" sakiot
  done
  database_url="$(sed -n 's/^DATABASE_URL=//p' "${production_env}" | head -n 1)"
  if psql "${database_url}" -tAc 'SELECT 1' >/dev/null 2>&1; then
    log "ok: ${production_env} reaches the production database"
  else
    log "FAIL: ${production_env} cannot reach the production database"
    failed=1
  fi
  [[ "${failed}" -eq 0 ]] || die "production hardened, but the checks above failed"
  log "done. Check for sandbox denials: journalctl -u sakiot-web.service --since -10min | grep -i 'denied\|not permitted'"
  exit 0
fi

# ---- preflight ----------------------------------------------------------------
# An engine older than this change chmods the data directory on every deploy,
# which fails once staging's runtime user owns it.
installed_tree="$(cat "${install_root}/engine-src-tree" 2>/dev/null || true)"
checkout_tree="$(git -c safe.directory="${repo_root}" -C "${repo_root}" \
  rev-parse HEAD:ops/sakiot-deploy 2>/dev/null || true)"
[[ -n "${installed_tree}" && "${installed_tree}" == "${checkout_tree}" ]] \
  || die "the installed deploy engine was not built from this checkout; run ops/update-deploy-engine.sh first"
for file in "${production_env}" "${staging_env}"; do
  [[ -f "${file}" ]] || die "missing ${file}"
done
# Runtime users write with UMask=0007, which nginx (www-data) cannot read.
if grep -rqsE '/var/lib/sakiot[^ ;]*/data' /etc/nginx; then
  die "nginx references a data directory; serve media through the web server before isolating"
fi

production_db="$(identifier "$(url_part "${production_env}" database)")"
production_role="$(identifier "$(url_part "${production_env}" user)")"
staging_db="$(identifier "$(url_part "${staging_env}" database)")"

slots=()
for unit in /etc/systemd/system/sakiot-preview-*-web.service; do
  [[ -e "${unit}" ]] || continue
  slot="${unit#/etc/systemd/system/sakiot-preview-}"
  slot="${slot%-web.service}"
  [[ "${slot}" =~ ^[a-z0-9][a-z0-9-]{0,31}$ ]] && slots+=("${slot}")
done
if [[ "${#slots[@]}" -gt 0 && ! -f "${preview_env}" ]]; then
  die "preview slots exist but ${preview_env} is missing"
fi

hold_deploy_lock /var/lib/sakiot-staging/deploy
for slot in "${slots[@]}"; do
  hold_deploy_lock "/var/lib/sakiot-preview-${slot}/deploy"
done
mapfile -t staging_units < <(instance_units sakiot-staging /var/lib/sakiot-staging/deploy)

log "plan:"
log "  users: sakiot-staging and sakiot-preview; sakiot joins their groups"
log "  staging: role sakiot_staging owns ${staging_db}, closed to other roles;"
log "           ${staging_env} uses it; /var/lib/sakiot-staging/data belongs to sakiot-staging;"
log "           restarts ${staging_units[*]:-nothing} on the hardened units"
if [[ "${#slots[@]}" -gt 0 ]]; then
  log "  previews (${slots[*]}): role sakiot_slot, user sakiot-preview, via preview-slot.sh"
fi
log "  production: only ${production_role} (and superusers) may connect to ${production_db};"
log "              its services keep running untouched"
log "  env files are backed up next to them"
confirm
trap 'log "failed; staging may be stopped. Fix the error above and re-run: the script is idempotent"' ERR

# ---- runtime users ------------------------------------------------------------
for runtime_user in sakiot-staging sakiot-preview; do
  if ! id "${runtime_user}" >/dev/null 2>&1; then
    useradd --system --user-group --home-dir /nonexistent --no-create-home \
      --shell /usr/sbin/nologin "${runtime_user}"
    log "created user ${runtime_user}"
  fi
done
usermod -a -G sakiot-staging,sakiot-preview sakiot

# ---- database roles and env files ---------------------------------------------
# Points `env_file` at `role`. A file already using the role keeps its
# password; otherwise a new one is generated, because the old one belongs to
# the role production uses.
use_role() {
  local role="$1" env_file="$2" password
  if [[ "$(url_part "${env_file}" user)" == "${role}" ]]; then
    password="$(url_part "${env_file}" password)"
  else
    password="$(openssl rand -hex 24)"
  fi
  [[ "${password}" =~ ^[A-Za-z0-9._~-]+$ ]] \
    || die "the ${role} password in ${env_file} is not URL-safe; use letters and digits"
  set_role_password "${role}" "${password}"
  if [[ "$(url_part "${env_file}" user)" != "${role}" ]]; then
    "${ops_dir}/rewrite-db-role.sh" "${env_file}" "${role}" "${password}"
  fi
}
use_role sakiot_staging "${staging_env}"
if [[ -f "${preview_env}" ]]; then
  use_role sakiot_slot "${preview_env}"
fi

# ---- staging --------------------------------------------------------------------
if [[ "${#staging_units[@]}" -gt 0 ]]; then
  log "stopping ${staging_units[*]}"
  systemctl stop "${staging_units[@]}"
fi

own_database "${staging_db}" sakiot_staging
log "sakiot_staging owns ${staging_db}"

staging_data=/var/lib/sakiot-staging/data
install -d -o sakiot-staging -g sakiot-staging -m 2770 "${staging_data}"
chown -R -h sakiot-staging:sakiot-staging "${staging_data}"
chmod -R u+rwX,g+rwX,o-rwx "${staging_data}"
find "${staging_data}" -type d -exec chmod g+s {} +
chgrp sakiot-staging /srv/sakiot-staging/releases /srv/sakiot-staging/current
chmod 0750 /srv/sakiot-staging/releases /srv/sakiot-staging/current
log "${staging_data} belongs to sakiot-staging"

install_unit sakiot-staging-web.service
install_unit sakiot-staging-fbi-agent@.service
systemctl daemon-reload
if [[ "${#staging_units[@]}" -gt 0 ]]; then
  systemctl start "${staging_units[@]}"
  log "started ${staging_units[*]}"
fi

# ---- preview slots --------------------------------------------------------------
# preview-slot.sh is idempotent and applies the new layout, role, and unit;
# --no-dns leaves the existing records alone. The engine also copies a slot's
# DATABASE_URL, credentials included, into each release, where it overrides
# preview.env; those copies still name production's role.
if [[ "${#slots[@]}" -gt 0 ]]; then
  slot_password="$(url_part "${preview_env}" password)"
fi
for slot in "${slots[@]}"; do
  "${ops_dir}/preview-slot.sh" "${slot}" --no-dns
  for release_env in /srv/sakiot-preview-"${slot}"/releases/*/web/service.env; do
    if [[ ! -f "${release_env}" ]] || ! grep -q '^DATABASE_URL=' "${release_env}"; then
      continue
    fi
    "${ops_dir}/rewrite-db-role.sh" --no-backup "${release_env}" sakiot_slot "${slot_password}" >/dev/null
  done
  systemctl restart "sakiot-preview-${slot}-web.service"
  log "preview slot ${slot} runs as sakiot-preview"
done

# ---- production database --------------------------------------------------------
other_roles="$(psql_admin -tAc "
  SELECT string_agg(DISTINCT usename, ', ')
    FROM pg_stat_activity
   WHERE datname = '${production_db}' AND usename NOT IN ('${production_role}', 'postgres')")"
if [[ -n "${other_roles}" ]]; then
  log "warning: ${other_roles} connected to ${production_db}; left CONNECT open. Grant them CONNECT, then re-run"
else
  psql_admin -d postgres \
    -c "REVOKE CONNECT, TEMPORARY ON DATABASE ${production_db} FROM PUBLIC" \
    -c "GRANT CONNECT, TEMPORARY ON DATABASE ${production_db} TO ${production_role}"
  log "only ${production_role} and superusers can connect to ${production_db}"
fi

# ---- verification -----------------------------------------------------------------
trap - ERR
sleep 5
for unit in "${staging_units[@]}"; do
  check_unit "${unit}" sakiot-staging
done
for slot in "${slots[@]}"; do
  check_unit "sakiot-preview-${slot}-web.service" sakiot-preview
done
if sudo -u sakiot-staging test -r "${production_env}"; then
  log "FAIL: sakiot-staging can read ${production_env}"
  failed=1
else
  log "ok: sakiot-staging cannot read ${production_env}"
fi
[[ "${failed}" -eq 0 ]] || die "isolation applied, but the checks above failed"
log "done. Once staging has run well, harden production: $0 --harden-production"

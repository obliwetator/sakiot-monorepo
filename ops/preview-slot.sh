#!/usr/bin/env bash
set -euo pipefail

# Bootstraps or tears down one preview slot on the VPS: Cloudflare DNS record,
# shared env file, database, systemd unit, nginx vhost, and HTTPS cert. Run as
# root from a monorepo checkout. Adding a slot is the only manual part of
# hosting a branch preview; the deploy itself (`Deploy preview` workflow)
# never touches this script.
#
# All slots share /etc/sakiot/preview.env; the deploy engine derives each
# slot's port, database, dirs, and domain from the slot name. The web port is
# deterministic (8903 + hash(slot)), matching ops/sakiot-deploy's slot_port().
#
#   ops/preview-slot.sh <slot>              # create slot
#   ops/preview-slot.sh <slot> --remove     # tear the slot down (also purges
#                                           # B2 objects the slot created when
#                                           # a purge key is configured)
#   ops/preview-slot.sh <slot> --no-dns     # skip Cloudflare (wildcard DNS in place)
#
# Environment:
#   CLOUDFLARE_API_TOKEN   Zone:DNS edit token (needed unless --no-dns);
#                          defaults to the value in the shared preview.env
#   PREVIEW_DOMAIN         default: preview.patrykstyla.com
#   VPS_IP                 default: detected via api.ipify.org
#   CERTBOT_EMAIL          override; default comes from the shared preview.env

OPS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${OPS_DIR}/.." && pwd)"
SLOT="${1:-}"
ACTION=create
NO_DNS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --remove) ACTION=remove ;;
        --no-dns) NO_DNS=1 ;;
        *) SLOT="$1" ;;
    esac
    shift
done

die() { printf '\033[1;31m[preview-slot]\033[0m %s\n' "$*" >&2; exit 1; }
log() { printf '\033[1;34m[preview-slot]\033[0m %s\n' "$*"; }

[[ -n "$SLOT" ]] || die "usage: ops/preview-slot.sh <slot> [--remove|--no-dns]"
[[ "$SLOT" =~ ^[a-z0-9][a-z0-9-]{0,31}$ ]] || die "invalid slot name '$SLOT'"

DOMAIN="${PREVIEW_DOMAIN:-preview.patrykstyla.com}"
SUBDOMAIN="${SLOT}.${DOMAIN}"
[[ "$(id -u)" -eq 0 ]] || die "run as root"

# The shared env file holds CLOUDFLARE_API_TOKEN and CERTBOT_EMAIL; fall back
# to it when the variables were not passed on the command line.
ENV_FILE="/etc/sakiot/preview.env"
if [[ -z "${CLOUDFLARE_API_TOKEN:-}" && -f "$ENV_FILE" ]]; then
    CLOUDFLARE_API_TOKEN="$(sed -n 's/^CLOUDFLARE_API_TOKEN=//p' "$ENV_FILE" | head -n1)"
fi

# Deterministic web port; must stay in sync with slot_port() in
# ops/sakiot-deploy/src/config.rs.
slot_port() {
    local slot="$1" h=0 i c
    for ((i = 0; i < ${#slot}; i++)); do
        printf -v c '%d' "'${slot:i:1}"
        h=$(( (h * 31 + c) % 4294967296 ))
    done
    printf '%d' $(( 8903 + h % 25 ))
}

if [[ "$ACTION" = create ]]; then
    # ---- DNS record --------------------------------------------------------
    if [[ "$NO_DNS" -eq 0 ]]; then
        : "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN (Zone:DNS edit) or use --no-dns}"
        zone="${CLOUDFLARE_ZONE:-${DOMAIN#*.}}"
        ip="${VPS_IP:-$(curl -4 -fsS --max-time 10 https://api.ipify.org || die "could not detect VPS IP; set VPS_IP")}"
        log "ensuring A record ${SUBDOMAIN} -> ${ip} (zone ${zone})"
        zone_id=$(curl -fsS --max-time 15 "https://api.cloudflare.com/client/v4/zones?name=${zone}" \
            -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" | jq -r '.result[0].id // empty')
        [[ -n "$zone_id" ]] || die "Cloudflare zone '${zone}' not found"
        existing=$(curl -fsS --max-time 15 \
            "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records?type=A&name=${SUBDOMAIN}" \
            -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" | jq -r '.result[0].id // empty')
        if [[ -n "$existing" ]]; then
            curl -fsS --max-time 15 -X PATCH \
                "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records/${existing}" \
                -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" -H "Content-Type: application/json" \
                -d "{\"content\":\"${ip}\"}" >/dev/null
        else
            curl -fsS --max-time 15 -X POST \
                "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records" \
                -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" -H "Content-Type: application/json" \
                -d "{\"type\":\"A\",\"name\":\"${SUBDOMAIN}\",\"content\":\"${ip}\",\"proxied\":false}" \
                >/dev/null
        fi
    else
        log "skipping DNS (--no-dns); ${SUBDOMAIN} must already resolve"
    fi

    # ---- web port (deterministic, matches the engine's slot_port) ----------
    PORT=$(slot_port "$SLOT")
    log "web port: ${PORT} (deterministic for slot '${SLOT}')"

    # ---- shared env file (created once, reused by every slot) ---------------
    if [[ -f "$ENV_FILE" ]]; then
        log "${ENV_FILE} already exists; keeping it"
    else
        repo_url=$(git -C "$ROOT" remote get-url origin 2>/dev/null || echo "https://github.com/OWNER/REPOSITORY.git")
        repo_url=${repo_url/git@github.com:/https:\/\/github.com\/}
        repo_url=${repo_url%.git}
        sed -e "s|OWNER/REPOSITORY|${repo_url#https://github.com/}|g" \
            "$OPS_DIR/preview.env.example" > "$ENV_FILE"
        log "wrote ${ENV_FILE} — set DEV_ACCOUNT_ID + DEV_LOGIN_SECRET (the only login is dev login; DISCORD_CLIENT_ID/SECRET are placeholders)"
    fi
    # The deploy engine reads this file as the sakiot user, so it must be
    # group-readable by sakiot; enforced on every run (heals older installs
    # that created it root:root and thus were unreadable for the engine).
    chown root:sakiot "$ENV_FILE"
    chmod 0640 "$ENV_FILE"

    # The engine and web server require every key the template defines (the
    # values may be placeholders, but a missing key silently falls back to
    # production defaults or fails the build). Fail loudly instead.
    missing_keys=()
    while IFS= read -r key; do
        grep -q "^${key}=" "$ENV_FILE" || missing_keys+=("$key")
    done < <(sed -n 's/^\([A-Z][A-Z0-9_]*\)=.*/\1/p' "$OPS_DIR/preview.env.example")
    if [[ "${#missing_keys[@]}" -gt 0 ]]; then
        die "missing from ${ENV_FILE}: ${missing_keys[*]}"
    fi

    # ---- per-slot directories (mirrors install-production.sh; the deploy
    # engine, running as sakiot, expects these to exist and be writable) -----
    install -d -o sakiot -g sakiot -m 0750 \
        "/var/lib/sakiot-preview-${SLOT}/data" \
        "/var/lib/sakiot-preview-${SLOT}/deploy" \
        "/var/lib/sakiot-preview-${SLOT}/backups" \
        "/srv/sakiot-preview-${SLOT}/releases" \
        "/srv/sakiot-preview-${SLOT}/current" \
        "/var/cache/sakiot-preview-${SLOT}"
    install -d -o sakiot -g sakiot -m 0755 "/var/www/${SUBDOMAIN}"
    log "created per-slot directories for ${SLOT}"

    # ---- database role (shared by every instance; created once) ------------
    db_pass="$(sed -n 's|^DATABASE_URL=postgres://[^:]*:\([^@]*\)@.*|\1|p' "$ENV_FILE" | head -n1)"
    if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='sakiot'" | grep -q 1; then
        [[ -n "$db_pass" ]] || die "could not read the database password from ${ENV_FILE} (DATABASE_URL)"
        sudo -u postgres psql -v ON_ERROR_STOP=1 \
            -c "CREATE ROLE sakiot LOGIN PASSWORD '${db_pass//\'/\'\'}'" >/dev/null
        log "created role sakiot"
    fi
    if [[ "$db_pass" == "replace_me" ]]; then
        log "warning: DATABASE_URL in ${ENV_FILE} still uses the 'replace_me' password; set a real one or deploys will fail"
    fi

    # ---- database ----------------------------------------------------------
    db="sakiot_preview_${SLOT}"
    db_created=0
    if sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='${db}'" | grep -q 1; then
        log "database ${db} already exists"
    else
        sudo -u postgres createdb -O sakiot "$db"
        log "created database ${db}"
        db_created=1
    fi

    # ---- copy staging data into brand-new slots ----------------------------
    # A fresh preview DB is empty and useless for testing real flows; seed it
    # with a snapshot of the staging database and the staging data files so
    # branch previews start from current state. Only on first creation;
    # re-running preview-up never clobbers slot-local data.
    if [[ "$db_created" -eq 1 ]]; then
        if sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='sakiot_staging'" | grep -q 1; then
            sudo -u postgres bash -c \
                "pg_dump -Fc sakiot_staging | pg_restore -d '${db}' --no-privileges"
            # Staging connects to its database as the postgres superuser, so
            # the restored tables are postgres-owned; the preview slot
            # connects as sakiot, which gets nothing without explicit grants
            # (future migrations run as sakiot and keep their own objects).
            sudo -u postgres psql -v ON_ERROR_STOP=1 -d "$db" -c \
                "GRANT USAGE ON SCHEMA public TO sakiot; \
                 GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO sakiot; \
                 GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO sakiot;"
            log "copied sakiot_staging database into ${db}"
        fi
        if [[ -d /var/lib/sakiot-staging/data ]]; then
            cp -a /var/lib/sakiot-staging/data/. "/var/lib/sakiot-preview-${SLOT}/data/"
            log "copied staging data files into the preview slot"
        fi
    fi

    # ---- dev-login account --------------------------------------------------
    # Preview databases start empty, but GET /api/users/current 500s on a
    # missing row for DEV_ACCOUNT_ID (RowNotFound), so dev login breaks until
    # the account is inserted. Seed it on every run (idempotent).
    dev_account="$(sed -n 's/^DEV_ACCOUNT_ID=//p' "$ENV_FILE" | head -n1)"
    if [[ "$dev_account" =~ ^[0-9]+$ && "$dev_account" != "0" ]]; then
        sudo -u postgres psql -v ON_ERROR_STOP=1 -d "$db" -c \
            "INSERT INTO discord_auth_user (id, username, discriminator, avatar, flags, public_flags) \
             VALUES ($dev_account, 'dev', '0', '', 0, 0) ON CONFLICT (id) DO NOTHING" >/dev/null
        log "seeded dev-login account ${dev_account} in ${db}"
    else
        log "warning: DEV_ACCOUNT_ID missing or invalid in ${ENV_FILE}; dev login will fail"
    fi

    # ---- systemd unit (web only; preview slots run no FBI Agent) ------------
    sed \
        -e "s|sakiot-staging|sakiot-preview-${SLOT}|g" \
        -e "s|/etc/sakiot/staging.env|${ENV_FILE}|g" \
        "$OPS_DIR/systemd/sakiot-staging-web.service" > "/etc/systemd/system/sakiot-preview-${SLOT}-web.service"
    log "installed unit sakiot-preview-${SLOT}-web.service"
    systemctl daemon-reload
    systemctl enable "sakiot-preview-${SLOT}-web.service" >/dev/null 2>&1 || true

    # ---- nginx -------------------------------------------------------------
    vhost="/etc/nginx/sites-available/${SUBDOMAIN}"
    sed \
        -e "s|SLOT|${SLOT}|g" \
        -e "s|PORT|${PORT}|g" \
        "$OPS_DIR/nginx/preview-slot.conf.example" > "$vhost"
    ln -sfn "$vhost" "/etc/nginx/sites-enabled/${SUBDOMAIN}"
    nginx -t >/dev/null && systemctl reload nginx
    log "installed nginx vhost ${SUBDOMAIN}"

    # ---- HTTPS -------------------------------------------------------------
    certbot_email="${CERTBOT_EMAIL:-$(sed -n 's/^CERTBOT_EMAIL=//p' "$ENV_FILE" 2>/dev/null | head -n1)}"
    if [[ -n "$certbot_email" ]] && command -v certbot >/dev/null 2>&1; then
        if certbot --nginx -d "$SUBDOMAIN" --non-interactive --agree-tos -m "$certbot_email" >/dev/null 2>&1; then
            log "issued HTTPS certificate for ${SUBDOMAIN}"
        else
            log "certbot failed for ${SUBDOMAIN}; slot stays on HTTP"
        fi
    elif [[ -z "$certbot_email" ]]; then
        log "no CERTBOT_EMAIL (shared ${ENV_FILE} or env); skipping HTTPS"
    fi

    log "slot ${SLOT} ready: https://${SUBDOMAIN} (after dev-login creds in ${ENV_FILE} are set)"
    log "branches whose slug is '${SLOT}' now auto-deploy on push and tear down on delete"
    log "or deploy it manually: Actions -> Deploy preview -> slot=${SLOT}, branch=<branch>"

elif [[ "$ACTION" = remove ]]; then
    # ---- DNS ---------------------------------------------------------------
    if [[ "$NO_DNS" -eq 0 ]]; then
        : "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN or use --no-dns}"
        zone="${CLOUDFLARE_ZONE:-${DOMAIN#*.}}"
        zone_id=$(curl -fsS --max-time 15 "https://api.cloudflare.com/client/v4/zones?name=${zone}" \
            -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" | jq -r '.result[0].id // empty')
        if [[ -n "$zone_id" ]]; then
            record_id=$(curl -fsS --max-time 15 \
                "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records?type=A&name=${SUBDOMAIN}" \
                -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" | jq -r '.result[0].id // empty')
            if [[ -n "$record_id" ]]; then
                curl -fsS --max-time 15 -X DELETE \
                    "https://api.cloudflare.com/client/v4/zones/${zone_id}/dns_records/${record_id}" \
                    -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" >/dev/null
                log "deleted DNS record ${SUBDOMAIN}"
            fi
        fi
    fi

    # ---- cert --------------------------------------------------------------
    command -v certbot >/dev/null 2>&1 && certbot delete --cert-name "$SUBDOMAIN" >/dev/null 2>&1 || true

    # ---- nginx -------------------------------------------------------------
    rm -f "/etc/nginx/sites-enabled/${SUBDOMAIN}" "/etc/nginx/sites-available/${SUBDOMAIN}"
    nginx -t >/dev/null 2>&1 && systemctl reload nginx || true
    log "removed nginx vhost ${SUBDOMAIN}"

    # ---- systemd -----------------------------------------------------------
    systemctl disable --now "sakiot-preview-${SLOT}-web.service" >/dev/null 2>&1 || true
    systemctl stop "sakiot-preview-${SLOT}-fbi-agent@*" >/dev/null 2>&1 || true
    rm -f \
        "/etc/systemd/system/sakiot-preview-${SLOT}-web.service" \
        "/etc/systemd/system/sakiot-preview-${SLOT}-fbi-agent@.service"
    systemctl daemon-reload
    log "removed systemd units"

    # ---- local data files --------------------------------------------------
    # The slot's recording/clip/waveform files are its own copies; removing
    # them never touches staging or production data.
    if [[ -d "/var/lib/sakiot-preview-${SLOT}/data" ]]; then
        rm -rf "/var/lib/sakiot-preview-${SLOT}/data"
        log "removed preview data files"
    fi

    # ---- B2 objects uploaded by the slot -----------------------------------
    # Purges automatically whenever a delete-capable key is configured:
    # /etc/sakiot/preview-b2-purge.env (root-only, holds B2_PURGE_KEY_ID and
    # B2_PURGE_KEY_SECRET) or B2_PURGE_KEY_ID/SECRET env vars. The runtime
    # media key lacks deleteFiles by design, so purge needs its own key; the
    # web server and deploy engine never see it. Only keys present in the
    # slot DB but absent from staging's media_objects were created by the
    # slot; rclone deletefile only hides the current version, so retained
    # B2 versions stay recoverable by an admin.
    purge_key_id="${B2_PURGE_KEY_ID:-}"
    purge_key_secret="${B2_PURGE_KEY_SECRET:-}"
    purge_env="/etc/sakiot/preview-b2-purge.env"
    if [[ -f "$purge_env" ]]; then
        [[ -n "$purge_key_id" ]] || purge_key_id="$(sed -n 's/^B2_PURGE_KEY_ID=//p' "$purge_env" | head -n1)"
        [[ -n "$purge_key_secret" ]] || purge_key_secret="$(sed -n 's/^B2_PURGE_KEY_SECRET=//p' "$purge_env" | head -n1)"
    fi
    if [[ -n "$purge_key_id" && -n "$purge_key_secret" ]]; then
        if ! command -v rclone >/dev/null 2>&1; then
            log "warning: rclone missing; skipping B2 purge for ${SLOT}"
        else
            b2_endpoint="$(sed -n 's/^SAKIOT_MEDIA_S3_ENDPOINT=//p' "$ENV_FILE" 2>/dev/null | head -n1)"
            b2_bucket="$(sed -n 's/^SAKIOT_MEDIA_S3_BUCKET=//p' "$ENV_FILE" 2>/dev/null | head -n1)"
            preview_db="sakiot_preview_${SLOT}"
            if [[ -z "$b2_bucket" ]] || [[ ! -f "$ENV_FILE" ]] \
                || ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='${preview_db}'" | grep -q 1 \
                || ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='sakiot_staging'" | grep -q 1; then
                log "warning: cannot diff media keys (bucket or databases missing); skipping B2 purge for ${SLOT}"
            else
                count=0
                while IFS= read -r key; do
                    [[ -n "$key" ]] || continue
                    RCLONE_CONFIG_PURGE_TYPE=b2 \
                    RCLONE_CONFIG_PURGE_ACCOUNT="$purge_key_id" \
                    RCLONE_CONFIG_PURGE_KEY="$purge_key_secret" \
                        rclone deletefile "purge:${b2_bucket}/${key}" >/dev/null 2>&1 \
                        || log "warning: failed to delete b2 object ${key}"
                    count=$((count + 1))
                done < <(comm -23 \
                    <(sudo -u postgres psql -tAc \
                        "SELECT object_key FROM ${preview_db}.media_objects WHERE object_key IS NOT NULL" | sort -u) \
                    <(sudo -u postgres psql -tAc \
                        "SELECT object_key FROM sakiot_staging.media_objects WHERE object_key IS NOT NULL" | sort -u))
                log "purged ${count} slot-created B2 objects from ${b2_bucket}"
            fi
        fi
    else
        log "B2 purge not configured; skipping (set credentials in /etc/sakiot/preview-b2-purge.env to automate)"
    fi

    # ---- database ----------------------------------------------------------
    if sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='sakiot_preview_${SLOT}'" | grep -q 1; then
        sudo -u postgres dropdb "sakiot_preview_${SLOT}"
        log "dropped database sakiot_preview_${SLOT}"
    fi

    # ---- env file (shared: never removed with a slot) ----------------------
    rm -f "/var/lib/sakiot-preview-${SLOT}/deploy/current" 2>/dev/null || true
    log "shared ${ENV_FILE} kept (used by every slot)"
    log "slot ${SLOT} removed"
else
    die "unknown action '${ACTION}'"
fi

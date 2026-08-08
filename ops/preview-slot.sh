#!/usr/bin/env bash
set -euo pipefail

# Bootstraps or tears down one preview slot on the VPS: Cloudflare DNS record,
# env file, database, systemd units, nginx vhost, and HTTPS cert. Run as root
# from a monorepo checkout. Adding a slot is the only manual part of hosting a
# branch preview; the deploy itself (`Deploy preview` workflow) never touches
# this script.
#
#   ops/preview-slot.sh <slot>              # create slot
#   ops/preview-slot.sh <slot> --remove     # tear the slot down
#   ops/preview-slot.sh <slot> --no-dns     # skip Cloudflare (wildcard DNS in place)
#   ops/preview-slot.sh <slot> --port 8904  # pick the web port explicitly
#
# Environment:
#   CLOUDFLARE_API_TOKEN   Zone:DNS edit token (needed unless --no-dns)
#   PREVIEW_DOMAIN         default: preview.patrykstyla.com
#   VPS_IP                 default: detected via api.ipify.org
#   CERTBOT_EMAIL          set to also request an HTTPS cert for the slot

OPS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${OPS_DIR}/.." && pwd)"
SLOT="${1:-}"
ACTION=create
PORT=""
NO_DNS=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --remove) ACTION=remove ;;
        --no-dns) NO_DNS=1 ;;
        --port) PORT="$2"; shift ;;
        *) SLOT="$1" ;;
    esac
    shift
done

die() { printf '\033[1;31m[preview-slot]\033[0m %s\n' "$*" >&2; exit 1; }
log() { printf '\033[1;34m[preview-slot]\033[0m %s\n' "$*"; }

[[ -n "$SLOT" ]] || die "usage: ops/preview-slot.sh <slot> [--remove|--no-dns|--port N]"
[[ "$SLOT" =~ ^[a-z0-9][a-z0-9-]{0,31}$ ]] || die "invalid slot name '$SLOT'"

DOMAIN="${PREVIEW_DOMAIN:-preview.patrykstyla.com}"
SUBDOMAIN="${SLOT}.${DOMAIN}"
[[ -n "${PORT:-}" && ! "$PORT" =~ ^[0-9]+$ ]] && die "invalid --port '$PORT'"
[[ "$(id -u)" -eq 0 ]] || die "run as root"

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

    # ---- web port ----------------------------------------------------------
    if [[ -z "$PORT" ]]; then
        PORT=$(grep -h '^PORT=' /etc/sakiot/preview-*.env 2>/dev/null | cut -d= -f2 \
            | sort -n | tail -n1 | awk '{print $1 + 1}' )
        [[ -n "$PORT" ]] || PORT=8903
        [[ "$PORT" -ge 8903 && "$PORT" -le 8929 ]] || die "slot port ${PORT} out of range"
    fi
    log "web port: ${PORT}"

    # ---- env file ----------------------------------------------------------
    env_file="/etc/sakiot/preview-${SLOT}.env"
    [[ -f "$env_file" ]] && die "${env_file} already exists; remove the slot first"
    repo_url=$(git -C "$ROOT" remote get-url origin 2>/dev/null || echo "https://github.com/OWNER/REPOSITORY.git")
    repo_url=${repo_url/git@github.com:/https:\/\/github.com\/}
    repo_url=${repo_url%.git}
    sed \
        -e "s|sakiot-preview-|sakiot-preview-${SLOT}-|g" \
        -e "s|sakiot_preview|sakiot_preview_${SLOT}|g" \
        -e "s|preview.patrykstyla.com|${SUBDOMAIN}|g" \
        -e "s|/var/lib/sakiot-preview|/var/lib/sakiot-preview-${SLOT}|g" \
        -e "s|/srv/sakiot-preview|/srv/sakiot-preview-${SLOT}|g" \
        -e "s|/var/cache/sakiot-preview|/var/cache/sakiot-preview-${SLOT}|g" \
        -e "s|/etc/sakiot/preview.env|${env_file}|g" \
        -e "s|8902|${PORT}|g" \
        -e "s|OWNER/REPOSITORY|${repo_url#https://github.com/}|g" \
        "$OPS_DIR/preview.env.example" > "$env_file"
    chmod 0640 "$env_file"
    log "wrote ${env_file} — set DISCORD_CLIENT_ID/SECRET (reuse the staging OAuth app) and add ${SUBDOMAIN}/api/discord_login to its redirect URIs"

    # ---- database ----------------------------------------------------------
    db="sakiot_preview_${SLOT}"
    if sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='${db}'" | grep -q 1; then
        log "database ${db} already exists"
    else
        sudo -u postgres createdb -O sakiot "$db"
        log "created database ${db}"
    fi

    # ---- systemd unit (web only; preview slots run no FBI Agent) ------------
    sed \
        -e "s|sakiot-staging|sakiot-preview-${SLOT}|g" \
        -e "s|/etc/sakiot/staging.env|${env_file}|g" \
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
    if [[ -n "${CERTBOT_EMAIL:-}" ]] && command -v certbot >/dev/null 2>&1; then
        if certbot --nginx -d "$SUBDOMAIN" --non-interactive --agree-tos -m "$CERTBOT_EMAIL" >/dev/null 2>&1; then
            log "issued HTTPS certificate for ${SUBDOMAIN}"
        else
            log "certbot failed for ${SUBDOMAIN}; slot stays on HTTP"
        fi
    fi

    log "slot ${SLOT} ready: https://${SUBDOMAIN} (after OAuth creds in ${env_file} are set)"
    log "deploy it: Actions -> Deploy preview -> slot=${SLOT}, branch=<branch>"

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

    # ---- database ----------------------------------------------------------
    if sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='sakiot_preview_${SLOT}'" | grep -q 1; then
        sudo -u postgres dropdb "sakiot_preview_${SLOT}"
        log "dropped database sakiot_preview_${SLOT}"
    fi

    # ---- env file ----------------------------------------------------------
    env_file="/etc/sakiot/preview-${SLOT}.env"
    rm -f "$env_file" "/var/lib/sakiot-preview-${SLOT}/deploy/current" 2>/dev/null || true
    log "removed ${env_file}"
    log "slot ${SLOT} removed"
else
    die "unknown action '${ACTION}'"
fi

#!/usr/bin/env bash
# Regenerate or verify SQLx's checked-in query metadata against a clean schema.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-prepare}"
POSTGRES_IMAGE="${SQLX_POSTGRES_IMAGE:-postgres:18-alpine}"
CONTAINER_NAME="sakiot-sqlx-prepare-$$"
CONTAINER_ID=""

case "$MODE" in
    prepare) prepare_args=() ;;
    --check|check) prepare_args=(--check) ;;
    *)
        echo "usage: scripts/sqlx-prepare.sh [prepare|check]" >&2
        exit 2
        ;;
esac

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "error: missing dependency: $1 ($2)" >&2
        exit 1
    }
}

cleanup() {
    if [ -n "$CONTAINER_ID" ]; then
        docker rm -f "$CONTAINER_ID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT INT TERM

need docker "https://docs.docker.com/engine/install/"
if ! cargo sqlx --version >/dev/null 2>&1; then
    echo "error: sqlx-cli is required" >&2
    echo "install it with: cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,native-tls" >&2
    exit 1
fi

echo "Starting disposable PostgreSQL container..."
CONTAINER_ID=$(docker run --detach --rm \
    --name "$CONTAINER_NAME" \
    --publish 127.0.0.1::5432 \
    --env POSTGRES_DB=sakiot_sqlx \
    --env POSTGRES_USER=postgres \
    --env POSTGRES_PASSWORD=password \
    "$POSTGRES_IMAGE")

ready_checks=0
for _ in $(seq 1 60); do
    if docker exec "$CONTAINER_ID" pg_isready -U postgres -d sakiot_sqlx >/dev/null 2>&1; then
        ready_checks=$((ready_checks + 1))
        [ "$ready_checks" -ge 3 ] && break
    else
        # The image briefly starts and stops an initialization server before
        # the durable server is ready, so require consecutive successful probes.
        ready_checks=0
    fi
    sleep 1
done
if [ "$ready_checks" -lt 3 ]; then
    echo "error: disposable PostgreSQL did not become ready" >&2
    docker logs "$CONTAINER_ID" >&2
    exit 1
fi

port_mapping=$(docker port "$CONTAINER_ID" 5432/tcp | head -n 1)
port=${port_mapping##*:}
DATABASE_URL="postgres://postgres:password@127.0.0.1:${port}/sakiot_sqlx"

echo "Applying migrations..."
DATABASE_URL="$DATABASE_URL" SQLX_OFFLINE=false \
    cargo sqlx migrate run --source "$ROOT/sakiot-db/migrations"

if [ "$MODE" = "prepare" ]; then
    echo "Regenerating .sqlx metadata..."
else
    echo "Checking .sqlx metadata..."
fi
cd "$ROOT"
DATABASE_URL="$DATABASE_URL" SQLX_OFFLINE=false \
    cargo sqlx prepare "${prepare_args[@]}" --workspace -- --all-targets

#!/usr/bin/env bash
# Local debug environment for web_server. Runs on any dev machine.
#
#   scripts/dev.sh                 # up: db + migrate + seed + optional fixtures + cargo watch
#   scripts/dev.sh db              # only db + migrate + seed
#   scripts/dev.sh down            # stop local postgres
#   scripts/dev.sh reset           # drop local db volume, then db
#   scripts/dev.sh fetch-fixtures  # pull real recordings from staging (needs SSH)
#   scripts/dev.sh clean           # drop db volume + delete fetched fixture files
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE=(docker compose -f "$ROOT/compose.dev.yml")
LOCAL_DB_URL="postgres://postgres:password@localhost:54320/sakiot_rouvas"
DEFAULT_DEV_ACCOUNT_ID=999999999

log() { printf '\033[1;34m[dev]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[dev]\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1 ($2)"; }

env_get() { # env_get KEY [default]
    local val
    val=$(grep -E "^$1=" "$ROOT/.env" 2>/dev/null | head -n1 | cut -d= -f2- || true)
    printf '%s' "${val:-${2:-}}"
}

psql_local() {
    "${COMPOSE[@]}" exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d sakiot_rouvas "$@"
}

free_port() {
    # 8901 skipped on purpose: staging web uses it on the VPS.
    local p
    for p in 8900 8902 8903 8904 8905; do
        if ! (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; then
            printf '%s' "$p"
            return
        fi
    done
    die "no free port in 8900-8905; set PORT in .env manually"
}

ensure_env() {
    [ -f "$ROOT/.env" ] && return
    log "no .env found, generating one from .env.example with local-dev values"
    need openssl "usually preinstalled"
    local access refresh dev_secret port
    access=$(openssl rand -hex 32)
    refresh=$(openssl rand -hex 32)
    dev_secret=$(openssl rand -hex 16)
    port=$(free_port)
    [ "$port" != 8900 ] && log "port 8900 busy (production web on the VPS?), using $port"
    sed \
        -e "s|^DATABASE_URL=.*|DATABASE_URL=$LOCAL_DB_URL|" \
        -e "s|^SAKIOT_TEST_DATABASE_URL=.*|SAKIOT_TEST_DATABASE_URL=postgres://postgres:password@localhost:54320/sakiot_test|" \
        -e "s|^SAKIOT_DATA_DIR=.*|SAKIOT_DATA_DIR=$ROOT/data|" \
        -e "s|^JWT_ACCESS_SECRET=.*|JWT_ACCESS_SECRET=$access|" \
        -e "s|^JWT_REFRESH_SECRET=.*|JWT_REFRESH_SECRET=$refresh|" \
        -e "s|^DEV_ACCOUNT_ID=.*|DEV_ACCOUNT_ID=$DEFAULT_DEV_ACCOUNT_ID|" \
        -e "s|^DEV_LOGIN_SECRET=.*|DEV_LOGIN_SECRET=$dev_secret|" \
        -e "s|^VITE_DEV_LOGIN_SECRET=.*|VITE_DEV_LOGIN_SECRET=$dev_secret|" \
        -e "s|^PORT=.*|PORT=$port|" \
        -e "s|localhost:8900|localhost:$port|g" \
        -e "s|127.0.0.1:8900|127.0.0.1:$port|g" \
        "$ROOT/.env.example" > "$ROOT/.env"
    log "wrote $ROOT/.env (DISCORD_* left as placeholders; dev_login bypasses OAuth)"
}

ensure_frontend_env() {
    # .env.development (committed) points VITE_API_URL at staging; the
    # git-ignored .local file takes precedence in Vite's development mode.
    [ -f "$ROOT/.env.development.local" ] && return
    log "writing .env.development.local (points bun dev at the local web_server)"
    cat > "$ROOT/.env.development.local" <<EOF
VITE_API_URL=http://localhost:$(env_get PORT 8900)/api/
VITE_DEV_LOGIN_SECRET=$(env_get DEV_LOGIN_SECRET)
EOF
}

ensure_data_dirs() {
    local data_dir
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    mkdir -p "$data_dir"/{voice_recordings,no_silence_voice_recordings,waveform_data,clips}
    # recordings listing read_dirs the guild folder; missing dir = 404
    mkdir -p "$data_dir/voice_recordings/111111111111111111"
}

db_up() {
    need docker "https://docs.docker.com/engine/install/"
    if ! command -v sqlx >/dev/null 2>&1; then
        die "sqlx-cli missing. Install: cargo install sqlx-cli --no-default-features --features postgres,native-tls"
    fi
    ensure_env
    local db_url
    db_url=$(env_get DATABASE_URL "$LOCAL_DB_URL")
    if [ "$db_url" != "$LOCAL_DB_URL" ]; then
        die ".env DATABASE_URL points at '$db_url' but dev.sh manages the compose db at '$LOCAL_DB_URL'.
      Either edit .env to use the compose db, or back it up (mv .env .env.bak) and rerun to regenerate."
    fi
    log "starting postgres (compose.dev.yml)"
    "${COMPOSE[@]}" up -d --wait
    log "running migrations"
    DATABASE_URL="$(env_get DATABASE_URL "$LOCAL_DB_URL")" \
        sqlx migrate run --source "$ROOT/sakiot-db/migrations"
    log "seeding dev account + guild"
    psql_local -v dev_id="$(env_get DEV_ACCOUNT_ID "$DEFAULT_DEV_ACCOUNT_ID")" \
        -f - < "$ROOT/scripts/dev-seed.sql"
    ensure_frontend_env
    ensure_data_dirs
}

run_server() {
    if ! command -v cargo-watch >/dev/null 2>&1; then
        log "cargo-watch missing (auto-rebuild on save)."
        read -r -p "Install with 'cargo install cargo-watch'? [y/N] " yn
        case "$yn" in
            [Yy]*) cargo install cargo-watch ;;
            *) die "install cargo-watch or run manually: cd web_server && cargo run -p web_server --features dev-login" ;;
        esac
    fi
    log "starting web_server with dev-login on http://localhost:$(env_get PORT 8900) (cargo watch)"
    log "login secret: DEV_LOGIN_SECRET in .env (frontend picks up VITE_DEV_LOGIN_SECRET)"
    # cwd must be web_server/ so the relative callback.html path resolves
    cd "$ROOT/web_server"
    cargo watch -x 'run -p web_server --features dev-login'
}

stop_db_on_exit() {
    local status=$?
    trap - EXIT
    log "stopping local postgres (database volume is preserved)"
    "${COMPOSE[@]}" stop >/dev/null || true
    exit "$status"
}

cmd_up() {
    db_up
    trap stop_db_on_exit EXIT
    prompt_fetch_fixtures
    run_server
}

cmd_down() {
    "${COMPOSE[@]}" down
}

cmd_reset() {
    printf 'This drops the LOCAL dev database volume (sakiot-dev-postgres). Continue? [y/N] '
    read -r yn
    case "$yn" in
        [Yy]*) ;;
        *) die "aborted" ;;
    esac
    "${COMPOSE[@]}" down -v
    clear_fixture_media
    db_up
}

# --- fetch-fixtures: copy real recordings (rows + files) from staging ---------

sh_quote() {
    printf '%q' "$1"
}

is_uint() {
    [[ "$1" =~ ^[0-9]+$ ]]
}

probe_remote_psql() {
    local candidate=$1
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        bash -lc "$candidate -v ON_ERROR_STOP=1 -Atc 'SELECT 1'" >/dev/null 2>&1
    else
        ssh "$SAKIOT_DEV_SSH" "$candidate -v ON_ERROR_STOP=1 -Atc 'SELECT 1'" >/dev/null 2>&1
    fi
}

configure_remote_psql() {
    if [ -n "${SAKIOT_DEV_REMOTE_PSQL:-}" ]; then
        REMOTE_PSQL="$SAKIOT_DEV_REMOTE_PSQL -d $(sh_quote "$REMOTE_DB")"
        return
    fi

    local candidate env_file
    env_file=${SAKIOT_DEV_REMOTE_ENV_FILE:-/etc/sakiot/staging.env}
    for candidate in \
        "set -a; . $(sh_quote "$env_file"); set +a; psql \"\$DATABASE_URL\"" \
        "sudo -n -u postgres psql -d $(sh_quote "$REMOTE_DB")" \
        "psql -d $(sh_quote "$REMOTE_DB")"; do
        if probe_remote_psql "$candidate"; then
            REMOTE_PSQL=$candidate
            return
        fi
    done

    die "cannot connect to $REMOTE_DB on $SAKIOT_DEV_SSH.
      Grant the SSH user read access to $env_file, passwordless 'sudo -n -u postgres psql', or a matching Postgres role.
      Override with SAKIOT_DEV_REMOTE_PSQL if this VPS uses a different psql command."
}

fetch_tsv() { # fetch_tsv "<select query>"
    local copy_sql="COPY ($1) TO STDOUT;"
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        printf '%s\n' "$copy_sql" | bash -lc "$REMOTE_PSQL -v ON_ERROR_STOP=1 -At"
    else
        printf '%s\n' "$copy_sql" | ssh "$SAKIOT_DEV_SSH" "$REMOTE_PSQL -v ON_ERROR_STOP=1 -At"
    fi
}

import_tsv() { # import_tsv <table> <tsv-file> [fixup-sql run before the insert]
    [ -s "$2" ] || { log "  $1: nothing to import"; return; }
    {
        echo "BEGIN;"
        echo "CREATE TEMP TABLE _imp (LIKE $1 INCLUDING DEFAULTS);"
        echo "COPY _imp FROM STDIN;"
        cat "$2"
        echo "\\."
        [ -n "${3:-}" ] && echo "$3"
        echo "INSERT INTO $1 SELECT * FROM _imp ON CONFLICT DO NOTHING;"
        echo "COMMIT;"
    } | psql_local >/dev/null
    log "  $1: $(wc -l < "$2") row(s)"
}

fixture_count() {
    local data_dir recordings_manifest manifest
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    recordings_manifest="$data_dir/.dev-fixture-recordings.list"
    manifest="$data_dir/.dev-fixtures.list"
    if [ -f "$recordings_manifest" ]; then
        grep -c . "$recordings_manifest" || true
    elif [ -f "$manifest" ]; then
        grep -Ec '^voice_recordings/.+\.ogg$' "$manifest" || true
    else
        echo 0
    fi
}

clear_fixture_media() {
    local data_dir manifest recordings_manifest
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    manifest="$data_dir/.dev-fixtures.list"
    recordings_manifest="$data_dir/.dev-fixture-recordings.list"
    if [ -f "$manifest" ]; then
        while IFS= read -r f; do
            rm -f "$data_dir/$f"
        done < "$manifest"
        log "deleted $(wc -l < "$manifest") managed fixture file(s)"
    fi
    rm -f "$manifest" "$recordings_manifest"
    find "$data_dir" -depth -type d -empty -delete 2>/dev/null || true
}

replace_audio_fixtures() { # replace_audio_fixtures <old-names> <new-audio-tsv>
    {
        echo "BEGIN;"
        echo "CREATE TEMP TABLE _old_fixture_recordings (file_name text PRIMARY KEY);"
        echo "COPY _old_fixture_recordings FROM STDIN;"
        cat "$1"
        echo "\."
        echo "CREATE TEMP TABLE _imp (LIKE audio_files INCLUDING DEFAULTS);"
        echo "COPY _imp FROM STDIN;"
        cat "$2"
        echo "\."
        echo "UPDATE _imp SET recording_owner_instance_id = NULL, recording_heartbeat_at = NULL;"
        echo "DELETE FROM audio_files USING _old_fixture_recordings old WHERE audio_files.file_name = old.file_name AND NOT EXISTS (SELECT 1 FROM _imp new WHERE new.file_name = old.file_name);"
        echo "INSERT INTO audio_files SELECT * FROM _imp ON CONFLICT DO NOTHING;"
        echo "COMMIT;"
    } | psql_local >/dev/null
    log "  audio_files: replaced managed fixtures with $(wc -l < "$2") row(s)"
}

cmd_fetch_fixtures() (
    if [ -z "${SAKIOT_DEV_SSH:-}" ]; then
        SAKIOT_DEV_SSH=$(env_get SAKIOT_DEV_SSH)
    fi
    if [ -z "${SAKIOT_DEV_SSH:-}" ] && [ -d /var/lib/sakiot-staging/data ]; then
        log "staging data found on this machine, using local mode (no SSH)"
        SAKIOT_DEV_SSH=local
    fi
    if [ -z "${SAKIOT_DEV_SSH:-}" ] && [ -t 0 ]; then
        printf 'Staging SSH target (user@host): '
        read -r SAKIOT_DEV_SSH
    fi
    : "${SAKIOT_DEV_SSH:?set SAKIOT_DEV_SSH=user@vps-host (personal SSH access to the VPS), or 'local' on the VPS itself}"
    REMOTE_DB=${SAKIOT_DEV_REMOTE_DB:-sakiot_staging}
    local remote_data=${SAKIOT_DEV_REMOTE_DATA:-/var/lib/sakiot-staging/data}
    local count=20 guild_filter=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --count)
                [ $# -ge 2 ] || die "--count needs a value"
                is_uint "$2" || die "--count must be an unsigned integer"
                count=$2
                shift 2
                ;;
            --guild)
                [ $# -ge 2 ] || die "--guild needs a value"
                is_uint "$2" || die "--guild must be an unsigned integer"
                guild_filter="AND guild_id = $2"
                shift 2
                ;;
            *) die "unknown flag: $1 (supported: --count N, --guild ID)" ;;
        esac
    done
    [ -f "$ROOT/.env" ] || die "run 'scripts/dev.sh db' first"
    [ "$SAKIOT_DEV_SSH" = local ] || need ssh "for remote database export"
    need rsync "for file transfer"
    configure_remote_psql

    local tmp
    tmp=$(mktemp -d)
    trap 'rm -rf "${tmp:-}"' EXIT

    log "exporting up to $count recent recordings from $REMOTE_DB on $SAKIOT_DEV_SSH"
    fetch_tsv "SELECT * FROM audio_files WHERE reaped = false AND end_ts IS NOT NULL $guild_filter ORDER BY id DESC LIMIT $count" > "$tmp/audio_files.tsv"
    if [ ! -s "$tmp/audio_files.tsv" ]; then
        die "staging has no matching recordings — nothing to fetch"
    fi

    # audio_files columns: file_name guild_id channel_id user_id year month ...
    local guild_ids channel_ids user_ids
    guild_ids=$(cut -f2 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)
    channel_ids=$(cut -f3 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)
    user_ids=$(cut -f4 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)

    fetch_tsv "SELECT * FROM guilds WHERE id IN ($guild_ids)" > "$tmp/guilds.tsv"
    fetch_tsv "SELECT * FROM roles WHERE guild_id IN ($guild_ids)" > "$tmp/roles.tsv"
    fetch_tsv "SELECT * FROM channels WHERE channel_id IN ($channel_ids)" > "$tmp/channels.tsv"
    fetch_tsv "SELECT * FROM user_names WHERE user_id IN ($user_ids)" > "$tmp/user_names.tsv"
    fetch_tsv "SELECT * FROM user_nicknames WHERE user_id IN ($user_ids) AND guild_id IN ($guild_ids)" > "$tmp/user_nicknames.tsv"
    fetch_tsv "SELECT * FROM user_name_history WHERE user_id IN ($user_ids)" > "$tmp/user_name_history.tsv"
    # small lookup tables, populated at runtime on staging
    fetch_tsv "SELECT * FROM audio_file_finalize_reasons" > "$tmp/audio_file_finalize_reasons.tsv"
    fetch_tsv "SELECT * FROM user_name_event_types" > "$tmp/user_name_event_types.tsv"
    fetch_tsv "SELECT * FROM channel_type" > "$tmp/channel_type.tsv"

    # On-disk layout (sakiot-paths): voice_recordings/{g}/{c}/{YYYY}/{MM}/{stem}.ogg,
    # _no_silence_{stem}.ogg next to it, waveform_data/{stem}.dat (flat).
    awk -F'\t' '{
        dir = sprintf("%s/%s/%04d/%02d", $2, $3, $5, $6)
        printf "voice_recordings/%s/%s.ogg\n", dir, $1
        printf "no_silence_voice_recordings/%s/_no_silence_%s.ogg\n", dir, $1
        printf "waveform_data/%s.dat\n", $1
    }' "$tmp/audio_files.tsv" > "$tmp/files.list"
    cut -f1 "$tmp/audio_files.tsv" | sort -u > "$tmp/new-recordings.list"

    local data_dir manifest recordings_manifest src
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    manifest="$data_dir/.dev-fixtures.list"
    recordings_manifest="$data_dir/.dev-fixture-recordings.list"
    ensure_data_dirs

    # Download completely before changing the currently managed fixture set.
    mkdir -p "$tmp/media"
    src="$SAKIOT_DEV_SSH:$remote_data/"
    [ "$SAKIOT_DEV_SSH" = local ] && src="$remote_data/"
    log "downloading media files before replacing local fixtures"
    rsync -a --info=stats1 --ignore-missing-args \
        --files-from="$tmp/files.list" "$src" "$tmp/media/"

    while IFS= read -r f; do
        [ -f "$tmp/media/$f" ] && printf '%s\n' "$f"
    done < "$tmp/files.list" | sort -u > "$tmp/new-files.list"

    if [ -f "$recordings_manifest" ]; then
        sort -u "$recordings_manifest" > "$tmp/old-recordings.list"
    elif [ -f "$manifest" ]; then
        sed -n 's#^voice_recordings/.*/\([^/]*\)\.ogg$#\1#p' "$manifest" \
            | sort -u > "$tmp/old-recordings.list"
    else
        : > "$tmp/old-recordings.list"
    fi
    if [ -f "$manifest" ]; then
        sort -u "$manifest" > "$tmp/old-files.list"
    else
        : > "$tmp/old-files.list"
    fi

    log "importing rows into local db"
    import_tsv audio_file_finalize_reasons "$tmp/audio_file_finalize_reasons.tsv"
    import_tsv user_name_event_types "$tmp/user_name_event_types.tsv"
    import_tsv channel_type "$tmp/channel_type.tsv"
    import_tsv guilds "$tmp/guilds.tsv"
    import_tsv roles "$tmp/roles.tsv"
    import_tsv channels "$tmp/channels.tsv"
    import_tsv user_names "$tmp/user_names.tsv"
    import_tsv user_nicknames "$tmp/user_nicknames.tsv"
    import_tsv user_name_history "$tmp/user_name_history.tsv"
    # Deleting the old managed rows and inserting the new rows is one transaction.
    # Finalized recordings have no local live owner, so clear the staging owner.
    replace_audio_fixtures "$tmp/old-recordings.list" "$tmp/audio_files.tsv"

    local dev_id
    dev_id=$(env_get DEV_ACCOUNT_ID "$DEFAULT_DEV_ACCOUNT_ID")
    psql_local >/dev/null <<SQL
INSERT INTO guilds_present (guild_id)
    SELECT id FROM guilds WHERE id IN ($guild_ids) ON CONFLICT DO NOTHING;
INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
    SELECT id, $dev_id, 'Fixture guild ' || id, NULL, true, 8, '{}'
    FROM guilds WHERE id IN ($guild_ids) ON CONFLICT DO NOTHING;
SELECT setval('audio_files_id_seq', (SELECT COALESCE(MAX(id), 1) FROM audio_files));
SQL

    log "installing replacement media files into $data_dir"
    rsync -a "$tmp/media/" "$data_dir/"
    while IFS= read -r f; do
        rm -f "$data_dir/$f"
    done < <(comm -23 "$tmp/old-files.list" "$tmp/new-files.list")
    find "$data_dir" -depth -type d -empty -delete 2>/dev/null || true

    mv "$tmp/new-files.list" "$manifest"
    mv "$tmp/new-recordings.list" "$recordings_manifest"

    log "done: $(wc -l < "$tmp/audio_files.tsv") recording(s) from guild(s) $guild_ids"
)

prompt_fetch_fixtures() {
    [ -t 0 ] || return

    local count existing
    existing=$(fixture_count)
    while true; do
        printf 'Recordings to copy from staging (%s currently available; 0 keeps them) [0]: ' "$existing"
        read -r count
        count=${count:-0}
        if is_uint "$count"; then
            break
        fi
        echo "Enter an unsigned number, or 0 to skip." >&2
    done

    if [ "$count" -eq 0 ]; then
        log "skipping staging fixtures"
        return
    fi
    cmd_fetch_fixtures --count "$count"
}

cmd_clean() {
    local data_dir manifest
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    manifest="$data_dir/.dev-fixtures.list"
    echo "This will:"
    echo "  - stop postgres and drop the local dev db volume (sakiot-dev-postgres)"
    if [ -f "$manifest" ]; then
        echo "  - delete $(wc -l < "$manifest") fetched fixture file(s) under $data_dir"
    fi
    echo "  - remove the seeded guild dir and empty media dirs"
    echo "Other files in $data_dir are left alone. .env stays."
    printf 'Continue? [y/N] '
    read -r yn
    case "$yn" in
        [Yy]*) ;;
        *) die "aborted" ;;
    esac
    "${COMPOSE[@]}" down -v
    clear_fixture_media
    rmdir "$data_dir/voice_recordings/111111111111111111" 2>/dev/null || true
    find "$data_dir" -depth -type d -empty -delete 2>/dev/null || true
    log "clean done"
}

case "${1:-up}" in
    up) cmd_up ;;
    db) db_up ;;
    down) cmd_down ;;
    reset) cmd_reset ;;
    fetch-fixtures) shift; cmd_fetch_fixtures "$@" ;;
    clean) cmd_clean ;;
    help|-h|--help) sed -n '2,10p' "$0" ;;
    *) die "unknown command: $1 (up|db|down|reset|fetch-fixtures|clean)" ;;
esac

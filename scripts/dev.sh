#!/usr/bin/env bash
# Local debug environment for web_server. Runs on any dev machine.
#
#   scripts/dev.sh                 # up: db + migrate + seed + optional fixtures + cargo watch
#   scripts/dev.sh db              # only db + migrate + seed
#   scripts/dev.sh down            # stop local postgres
#   scripts/dev.sh reset           # drop local db volume, then db
#   scripts/dev.sh fetch-fixtures  # pull recent recordings from staging (needs SSH)
#   scripts/dev.sh fetch <what>    # pull one recording/session by dashboard URL or id
#   scripts/dev.sh clean           # drop db volume + delete fetched fixture files
#
# 'fetch' takes anything that identifies a recording: a full dashboard URL, the
# path part of one, a bare file name, or a bare numeric id.
#
#   scripts/dev.sh fetch https://patrykstyla.com/dashboard/362.../audio/763.../2026/7/17853-16117
#   scripts/dev.sh fetch https://patrykstyla.com/dashboard/362.../audio/session/384
#   scripts/dev.sh fetch 1785336946781-161172393719496704
#   scripts/dev.sh fetch --session 384 --source prod
#   scripts/dev.sh fetch <url> --into staging   # copy a prod recording into staging
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

ensure_media_tools() {
    need ffmpeg "install the FFmpeg package (Ubuntu/Debian: sudo apt install ffmpeg)"
    need ffprobe "installed alongside FFmpeg (Ubuntu/Debian: sudo apt install ffmpeg)"
    need audiowaveform "install audiowaveform (Ubuntu: sudo add-apt-repository ppa:chris-needham/ppa, then sudo apt update && sudo apt install audiowaveform)"
}

run_server() {
    if ! command -v cargo-watch >/dev/null 2>&1; then
        log "cargo-watch missing (auto-rebuild on save)."
        read -r -p "Install with 'cargo install cargo-watch'? [y/N] " yn
        case "$yn" in
            [Yy]*) cargo install cargo-watch ;;
            *) die "install cargo-watch or run manually: cd web-server && cargo run -p web_server --bin web_server --features dev-login" ;;
        esac
    fi
    log "starting web_server with dev-login on http://localhost:$(env_get PORT 8900) (cargo watch)"
    log "login secret: DEV_LOGIN_SECRET in .env (frontend picks up VITE_DEV_LOGIN_SECRET)"
    cd "$ROOT/web-server"
    cargo watch -x 'run -p web_server --bin web_server --features dev-login'
}

stop_db_on_exit() {
    local status=$?
    trap - EXIT
    log "stopping local postgres (database volume is preserved)"
    "${COMPOSE[@]}" stop >/dev/null || true
    exit "$status"
}

cmd_up() {
    ensure_media_tools
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

# --- fetch-fixtures: copy real recordings (rows + files) from a deployment ----

sh_quote() {
    printf '%q' "$1"
}

sql_quote() { # single-quoted SQL literal
    printf "'%s'" "${1//\'/\'\'}"
}

is_uint() {
    [[ "$1" =~ ^[0-9]+$ ]]
}

# Production and staging are two instances on the same VPS (see STAGING.md), so
# a source is just a set of four paths.
resolve_source() { # resolve_source prod|staging
    case "$1" in
        prod|production)
            SOURCE=prod
            SOURCE_LABEL=production
            REMOTE_DB=${SAKIOT_DEV_REMOTE_DB:-sakiot_rouvas}
            REMOTE_DATA=${SAKIOT_DEV_REMOTE_DATA:-/var/lib/sakiot/data}
            REMOTE_ENV_FILE=${SAKIOT_DEV_REMOTE_ENV_FILE:-/etc/sakiot/production.env}
            REMOTE_BINARY=${SAKIOT_DEV_REMOTE_WEB_BINARY:-/srv/sakiot/current/web/web_server}
            ;;
        staging)
            SOURCE=staging
            SOURCE_LABEL=staging
            REMOTE_DB=${SAKIOT_DEV_REMOTE_DB:-sakiot_staging}
            REMOTE_DATA=${SAKIOT_DEV_REMOTE_DATA:-/var/lib/sakiot-staging/data}
            REMOTE_ENV_FILE=${SAKIOT_DEV_REMOTE_ENV_FILE:-/etc/sakiot/staging.env}
            REMOTE_BINARY=${SAKIOT_DEV_REMOTE_WEB_BINARY:-/srv/sakiot-staging/current/web/web_server}
            ;;
        *) die "unknown source: $1 (supported: prod, staging)" ;;
    esac
}

source_for_host() { # empty when the host says nothing useful
    case "${1#www.}" in
        patrykstyla.com) printf prod ;;
        staging.patrykstyla.com|debug.patrykstyla.com) printf staging ;;
        *) printf '' ;;
    esac
}

# Accepts a full dashboard URL, the path part of one, a bare audio file name, or
# a bare numeric id. Sets SELECTOR_KIND/SELECTOR_VALUE, plus SELECTOR_HOST when
# the input carried one (used to guess the source).
parse_selector() {
    local token=$1 path host tail name
    SELECTOR_HOST=""
    path=$token
    case "$token" in
        http://*|https://*)
            path=${token#*://}
            host=${path%%/*}
            SELECTOR_HOST=${host%%:*}
            path=${path#"$host"}
            ;;
    esac
    path=${path%%\?*}
    path=${path%%#*}
    path=${path%/}

    case "$path" in
        */audio/session/*)
            name=${path##*/audio/session/}
            name=${name%%/*}
            is_uint "$name" || die "not a session id in '$token': '$name'"
            SELECTOR_KIND=session
            SELECTOR_VALUE=$name
            return
            ;;
        */audio/*)
            # <channel_id>/<year>/<month>/<file_name>
            tail=${path##*/audio/}
            [ "$(awk -F/ '{print NF}' <<<"$tail")" -eq 4 ] \
                || die "unrecognized audio URL: $token
      expected .../audio/<channel_id>/<year>/<month>/<file_name> or .../audio/session/<id>"
            name=${tail##*/}
            name=${name%.ogg}
            SELECTOR_KIND=file_name
            SELECTOR_VALUE=$name
            return
            ;;
    esac

    [ -n "$path" ] || die "empty recording selector"
    case "$path" in
        */*) die "unrecognized recording selector: $token" ;;
    esac
    if is_uint "$path"; then
        # audio_files.id and recording_sessions.id are separate sequences, so a
        # bare number is resolved against the source database.
        SELECTOR_KIND=numeric
        SELECTOR_VALUE=$path
        return
    fi
    [[ "$path" =~ ^[0-9A-Za-z._-]+$ ]] || die "unrecognized recording selector: $token"
    SELECTOR_KIND=file_name
    SELECTOR_VALUE=${path%.ogg}
}

resolve_numeric_selector() {
    [ "$SELECTOR_KIND" = numeric ] || return 0
    local as_recording as_session
    as_recording=$(fetch_tsv "SELECT 1 FROM audio_files WHERE id = $SELECTOR_VALUE")
    as_session=$(fetch_tsv "SELECT 1 FROM recording_sessions WHERE id = $SELECTOR_VALUE")
    if [ -n "$as_recording" ] && [ -n "$as_session" ]; then
        die "$SELECTOR_VALUE is both an audio_files.id and a recording_sessions.id in $REMOTE_DB.
      Disambiguate with --recording $SELECTOR_VALUE or --session $SELECTOR_VALUE."
    elif [ -n "$as_recording" ]; then
        SELECTOR_KIND=audio_id
    elif [ -n "$as_session" ]; then
        SELECTOR_KIND=session
    else
        die "$SELECTOR_VALUE matches no audio_files.id and no recording_sessions.id in $REMOTE_DB ($SOURCE_LABEL)"
    fi
    log "resolved $SELECTOR_VALUE to a $SELECTOR_KIND in $SOURCE_LABEL"
}

selector_predicate() {
    case "$SELECTOR_KIND" in
        file_name) printf 'file_name = %s' "$(sql_quote "$SELECTOR_VALUE")" ;;
        audio_id) printf 'id = %s' "$SELECTOR_VALUE" ;;
        session) printf 'recording_session_id = %s' "$SELECTOR_VALUE" ;;
        *) die "internal: unresolved selector kind '$SELECTOR_KIND'" ;;
    esac
}

# Connecting is not the same as being allowed to read: a peer-auth role can open
# the database and still hold no grant on a single table. These ask the server
# for the privilege itself, so a candidate is only accepted when the work will
# actually succeed. Dollar-quoted so the whole probe survives the remote shell.
READ_PROBE_SQL='SELECT $$ok$$ WHERE has_table_privilege($$audio_files$$, $$SELECT$$) AND has_table_privilege($$recording_sessions$$, $$SELECT$$)'
WRITE_PROBE_SQL='SELECT $$ok$$ WHERE has_table_privilege($$audio_files$$, $$INSERT$$) AND has_table_privilege($$recording_sessions$$, $$INSERT$$)'

probe_remote_psql() { # probe_remote_psql <psql command> <probe sql>
    local candidate=$1 sql=$2 cmd out
    cmd="$candidate -v ON_ERROR_STOP=1 -Atc $(sh_quote "$sql")"
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        out=$(bash -lc "$cmd" 2>/dev/null) || return 1
    else
        # The quoted probe is intentionally evaluated on the VPS.
        # shellcheck disable=SC2029
        out=$(ssh -n "$SAKIOT_DEV_SSH" "$cmd" 2>/dev/null) || return 1
    fi
    [ "$out" = ok ]
}

configure_remote_psql() {
    if [ -n "${SAKIOT_DEV_REMOTE_PSQL:-}" ]; then
        REMOTE_PSQL="$SAKIOT_DEV_REMOTE_PSQL -d $(sh_quote "$REMOTE_DB")"
        return
    fi

    local candidate
    for candidate in \
        "set -a; . $(sh_quote "$REMOTE_ENV_FILE"); set +a; psql \"\$DATABASE_URL\"" \
        "sudo -n -u postgres psql -d $(sh_quote "$REMOTE_DB")" \
        "psql -d $(sh_quote "$REMOTE_DB")"; do
        if probe_remote_psql "$candidate" "$READ_PROBE_SQL"; then
            REMOTE_PSQL=$candidate
            return
        fi
    done

    die "no psql on $SAKIOT_DEV_SSH can read $REMOTE_DB ($SOURCE_LABEL).
      Tried, in order:
        1. the service env file  . $REMOTE_ENV_FILE  (needs read access to it)
        2. sudo -n -u postgres psql -d $REMOTE_DB    (needs passwordless sudo)
        3. psql -d $REMOTE_DB                        (needs SELECT granted to your role)
      Connecting is not enough: each candidate must hold SELECT on audio_files and
      recording_sessions, so a role that can log in but was never granted the tables
      is rejected here instead of failing mid-export.
      Fix one of the three, or override with SAKIOT_DEV_REMOTE_PSQL.
      To see which you have:  ssh $SAKIOT_DEV_SSH 'psql -d $REMOTE_DB -Atc \"SELECT has_table_privilege(''audio_files'',''SELECT'')\"'"
}

fetch_tsv() { # fetch_tsv "<select query>"
    local copy_sql="COPY ($1) TO STDOUT;"
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        printf '%s\n' "$copy_sql" | bash -lc "$REMOTE_PSQL -v ON_ERROR_STOP=1 -At"
    else
        printf '%s\n' "$copy_sql" | ssh "$SAKIOT_DEV_SSH" "$REMOTE_PSQL -v ON_ERROR_STOP=1 -At"
    fi
}

hydrate_remote_recordings() { # hydrate_remote_recordings <comma-separated audio_file ids>
    local ids=$1 command sudo_command
    if [ -n "${SAKIOT_DEV_REMOTE_HYDRATE:-}" ]; then
        command="$SAKIOT_DEV_REMOTE_HYDRATE $(sh_quote "$ids")"
    else
        command="set -a; . $(sh_quote "$REMOTE_ENV_FILE"); set +a; $(sh_quote "$REMOTE_BINARY") media restore --audio-file-ids $(sh_quote "$ids")"
    fi
    log "materializing selected remote-only $SOURCE_LABEL recordings"
    sudo_command="sudo -n -u sakiot /bin/bash -lc $(sh_quote "$command")"
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        bash -lc "$command" 2>/dev/null && return
        bash -lc "$sudo_command" && return
    else
        # The quoted hydration script is intentionally evaluated on the staging host.
        # shellcheck disable=SC2029
        ssh -n "$SAKIOT_DEV_SSH" "$command" 2>/dev/null && return
        # shellcheck disable=SC2029
        ssh -n "$SAKIOT_DEV_SSH" "$sudo_command" && return
    fi
    die "could not hydrate $SOURCE_LABEL archive media. Grant env/data access, passwordless sudo -u sakiot, or set SAKIOT_DEV_REMOTE_HYDRATE."
}

psql_dest() { # SQL on stdin, against the destination of this fetch
    if [ "$DEST" = staging ]; then
        if [ "$SAKIOT_DEV_SSH" = local ]; then
            bash -lc "$DEST_PSQL -v ON_ERROR_STOP=1"
        else
            # The quoted psql command is intentionally evaluated on the VPS.
            # shellcheck disable=SC2029
            ssh "$SAKIOT_DEV_SSH" "$DEST_PSQL -v ON_ERROR_STOP=1"
        fi
    else
        psql_local
    fi
}

dest_tsv() { # dest_tsv "<select query>"
    if [ "$DEST" = staging ]; then
        printf 'COPY (%s) TO STDOUT;\n' "$1" | psql_dest
    else
        printf 'COPY (%s) TO STDOUT;\n' "$1" | psql_local -At
    fi
}

configure_dest_psql() {
    [ "$DEST" = staging ] || return 0
    if [ -n "${SAKIOT_DEV_STAGING_PSQL:-}" ]; then
        DEST_PSQL=$SAKIOT_DEV_STAGING_PSQL
        return
    fi
    local candidate env_file db
    env_file=${SAKIOT_DEV_STAGING_ENV_FILE:-/etc/sakiot/staging.env}
    db=${SAKIOT_DEV_STAGING_DB:-sakiot_staging}
    # Unlike the read-only export this needs a role that can write, so the
    # service env file (sakiot role) is tried before any bare psql.
    for candidate in \
        "set -a; . $(sh_quote "$env_file"); set +a; psql \"\$DATABASE_URL\"" \
        "sudo -n -u sakiot /bin/bash -lc $(sh_quote "set -a; . $(sh_quote "$env_file"); set +a; psql \"\$DATABASE_URL\"")" \
        "psql -d $(sh_quote "$db")"; do
        if probe_remote_psql "$candidate" "$WRITE_PROBE_SQL"; then
            DEST_PSQL=$candidate
            return
        fi
    done
    die "no psql on $SAKIOT_DEV_SSH can write to $db.
      Tried, in order:
        1. the service env file  . $env_file  (needs read access to it)
        2. sudo -n -u sakiot with the same env file (needs passwordless sudo)
        3. psql -d $db                        (needs INSERT granted to your role)
      Each candidate must hold INSERT on audio_files and recording_sessions.
      Fix one of the three, or override with SAKIOT_DEV_STAGING_PSQL."
}

import_tsv() { # import_tsv <table> <tsv-file> [fixup-sql run before the insert]
    [ -s "$2" ] || { log "  $1: nothing to import"; return; }
    {
        echo "BEGIN;"
        echo "CREATE TEMP TABLE _imp (LIKE $1 INCLUDING DEFAULTS) ON COMMIT DROP;"
        echo "COPY _imp FROM STDIN;"
        cat "$2"
        echo "\\."
        [ -n "${3:-}" ] && echo "$3"
        echo "INSERT INTO $1 SELECT * FROM _imp ON CONFLICT DO NOTHING;"
        echo "COMMIT;"
    } | psql_dest >/dev/null
    log "  $1: $(wc -l < "$2") row(s)"
}

emit_temp_copy() { # emit_temp_copy <temp-table> <like-table> <tsv-file>
    echo "CREATE TEMP TABLE $1 (LIKE $2 INCLUDING DEFAULTS) ON COMMIT DROP;"
    [ -s "$3" ] || return 0
    echo "COPY $1 FROM STDIN;"
    cat "$3"
    echo "\\."
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

# Imports a logical selection (sessions + fragments + gaps + events) into the
# destination. A session keeps its source id when the destination has it free,
# so a /audio/session/<id> URL stays portable; audio_files.id is always reissued
# because nothing addresses a recording by it and other tables reference it.
# audio_files.file_name embeds the capture timestamp and the Discord user id, so
# it is stable across environments and is what makes re-imports a no-op.
import_selection() { # import_selection <tmpdir> [marker-details-json]
    local tmp=$1 marker=${2:-}
    {
        echo "BEGIN;"
        emit_temp_copy _imp_sessions recording_sessions "$tmp/recording_sessions.tsv"
        emit_temp_copy _imp_audio audio_files "$tmp/audio_files.tsv"
        emit_temp_copy _imp_gaps recording_gaps "$tmp/recording_gaps.tsv"
        emit_temp_copy _imp_events recording_session_events "$tmp/recording_session_events.tsv"

        # Live-ownership columns point at bot instances of the source deployment.
        echo "UPDATE _imp_sessions SET owner_instance_id = NULL;"
        echo "UPDATE _imp_audio SET recording_owner_instance_id = NULL, recording_heartbeat_at = NULL;"

        echo "CREATE TEMP TABLE _session_map (
                  old_id bigint PRIMARY KEY,
                  new_id bigint NOT NULL,
                  reused boolean NOT NULL DEFAULT false
              ) ON COMMIT DROP;"
        # A fragment already present pins the rest of its session to the
        # destination session it was imported into.
        echo "INSERT INTO _session_map (old_id, new_id, reused)
              SELECT DISTINCT ON (a.recording_session_id)
                     a.recording_session_id, existing.recording_session_id, true
                FROM _imp_audio a
                JOIN audio_files existing ON existing.file_name = a.file_name
               WHERE a.recording_session_id IS NOT NULL
                 AND existing.recording_session_id IS NOT NULL
              ON CONFLICT (old_id) DO NOTHING;"
        echo "DELETE FROM _imp_audio a USING audio_files existing
               WHERE existing.file_name = a.file_name;"
        echo "DELETE FROM _imp_sessions s
               WHERE EXISTS (SELECT 1 FROM _session_map m WHERE m.old_id = s.id)
                  OR NOT EXISTS (SELECT 1 FROM _imp_audio a WHERE a.recording_session_id = s.id);"
        # Keeping the source session id makes a /audio/session/<id> URL work
        # verbatim against the destination, the way a file URL already does.
        # Lifting the sequence above both what exists and what is about to be
        # claimed first is what stops a fallback id landing on an id this same
        # batch claims (import 5 and 6 where 5 is taken, and a naive nextval
        # hands back 6).
        echo "SELECT setval(pg_get_serial_sequence('public.recording_sessions', 'id'),
                            GREATEST((SELECT COALESCE(MAX(id), 0) FROM recording_sessions),
                                     (SELECT COALESCE(MAX(id), 0) FROM _imp_sessions), 1));"
        echo "INSERT INTO _session_map (old_id, new_id)
              SELECT s.id,
                     CASE WHEN EXISTS (SELECT 1 FROM recording_sessions d WHERE d.id = s.id)
                          THEN nextval(pg_get_serial_sequence('public.recording_sessions', 'id'))
                          ELSE s.id END
                FROM _imp_sessions s;"

        # Gaps and events of a session that already exists are already there.
        echo "DELETE FROM _imp_gaps g
               WHERE NOT EXISTS (SELECT 1 FROM _imp_sessions s WHERE s.id = g.recording_session_id);"
        echo "DELETE FROM _imp_events e
               WHERE NOT EXISTS (SELECT 1 FROM _imp_sessions s WHERE s.id = e.recording_session_id);"

        echo "UPDATE _imp_audio a SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = a.recording_session_id;"
        echo "UPDATE _imp_gaps g SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = g.recording_session_id;"
        echo "UPDATE _imp_events e SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = e.recording_session_id;"
        echo "UPDATE _imp_sessions s SET id = m.new_id
                FROM _session_map m WHERE m.old_id = s.id;"
        echo "UPDATE _imp_audio SET id = nextval(pg_get_serial_sequence('public.audio_files', 'id'));"
        echo "UPDATE _imp_gaps SET id = nextval(pg_get_serial_sequence('public.recording_gaps', 'id'));"
        echo "UPDATE _imp_events SET id = nextval(pg_get_serial_sequence('public.recording_session_events', 'id'));"

        echo "INSERT INTO recording_sessions SELECT * FROM _imp_sessions;"
        # Claimed ids are inserted explicitly, which leaves the sequence behind
        # them; without this the destination's own bot would eventually hand out
        # an id this import already took.
        echo "SELECT setval(pg_get_serial_sequence('public.recording_sessions', 'id'),
                            GREATEST((SELECT COALESCE(MAX(id), 0) FROM recording_sessions), 1));"
        echo "INSERT INTO audio_files SELECT * FROM _imp_audio;"
        echo "INSERT INTO recording_gaps SELECT * FROM _imp_gaps;"
        echo "INSERT INTO recording_session_events SELECT * FROM _imp_events;"

        # Voice state and connection events feed every timeline lane except
        # "recording", and all the markers on the single-recording view.
        emit_temp_copy _imp_vse_types voice_state_event_types "$tmp/voice_state_event_types.tsv"
        emit_temp_copy _imp_vse voice_state_events "$tmp/voice_state_events.tsv"
        emit_temp_copy _imp_vce voice_connection_events "$tmp/voice_connection_events.tsv"

        echo "UPDATE _imp_vce SET owner_instance_id = NULL;"
        # voice_state_event_types.id is handed out by a sequence at runtime, so
        # the same name has different ids per environment. Its unique name is
        # the only stable key; importing the source id would hit the FK or,
        # worse, silently relabel every event.
        echo "INSERT INTO voice_state_event_types (name)
              SELECT t.name FROM _imp_vse_types t
               WHERE NOT EXISTS (SELECT 1 FROM voice_state_event_types d WHERE d.name = t.name);"
        echo "UPDATE _imp_vse v SET event_type_id = d.id
                FROM _imp_vse_types t
                JOIN voice_state_event_types d ON d.name = t.name
               WHERE t.id = v.event_type_id;"

        # Neither table has a key that survives the trip, so re-imports are made
        # idempotent on what actually identifies an occurrence.
        echo "DELETE FROM _imp_vse v USING voice_state_events e
               WHERE e.guild_id = v.guild_id AND e.user_id = v.user_id
                 AND e.event_type_id = v.event_type_id AND e.occurred_at = v.occurred_at;"
        echo "DELETE FROM _imp_vce c USING voice_connection_events e
               WHERE e.operation_id = c.operation_id;"

        echo "UPDATE _imp_vse SET id = nextval(pg_get_serial_sequence('public.voice_state_events', 'id'));"
        echo "UPDATE _imp_vce SET id = nextval(pg_get_serial_sequence('public.voice_connection_events', 'id'));"
        echo "INSERT INTO voice_state_events SELECT * FROM _imp_vse;"
        echo "INSERT INTO voice_connection_events SELECT * FROM _imp_vce;"

        if [ -n "$marker" ]; then
            # Placed at started_at so the marker sits at offset 0 of the session
            # timeline instead of far past its end.
            echo "INSERT INTO recording_session_events
                      (recording_session_id, occurred_at, event_type, channel_id, details)
                  SELECT rs.id, rs.started_at, 'imported_from_production', rs.starting_channel_id,
                         $(sql_quote "$marker")::jsonb || jsonb_build_object('origin_session_id', m.old_id)
                    FROM _session_map m
                    JOIN recording_sessions rs ON rs.id = m.new_id
                   WHERE NOT EXISTS (
                             SELECT 1 FROM recording_session_events e
                              WHERE e.recording_session_id = rs.id
                                AND e.event_type = 'imported_from_production');"
        fi
        echo "COMMIT;"
    } | psql_dest >/dev/null
}

# Drops managed fixture rows that the new selection replaced, plus any session
# left without fragments. recording_gaps/_events cascade with the session.
prune_managed_fixtures() { # prune_managed_fixtures <keep-names> <managed-names>
    [ -s "$2" ] || return 0
    {
        echo "BEGIN;"
        echo "CREATE TEMP TABLE _keep (file_name text PRIMARY KEY) ON COMMIT DROP;"
        if [ -s "$1" ]; then
            echo "COPY _keep FROM STDIN;"
            cat "$1"
            echo "\\."
        fi
        echo "CREATE TEMP TABLE _managed (file_name text PRIMARY KEY) ON COMMIT DROP;"
        echo "COPY _managed FROM STDIN;"
        cat "$2"
        echo "\\."
        echo "CREATE TEMP TABLE _touched_sessions ON COMMIT DROP AS
              SELECT DISTINCT af.recording_session_id AS id
                FROM audio_files af
                JOIN _managed m ON m.file_name = af.file_name
               WHERE af.recording_session_id IS NOT NULL;"
        echo "DELETE FROM audio_files af USING _managed m
               WHERE af.file_name = m.file_name
                 AND NOT EXISTS (SELECT 1 FROM _keep k WHERE k.file_name = af.file_name);"
        echo "DELETE FROM recording_sessions rs USING _touched_sessions t
               WHERE rs.id = t.id
                 AND NOT EXISTS (SELECT 1 FROM audio_files af WHERE af.recording_session_id = rs.id)
                 AND NOT EXISTS (SELECT 1 FROM clips c WHERE c.recording_session_id = rs.id);"
        echo "COMMIT;"
    } | psql_dest >/dev/null
}

json_string() {
    local s=$1
    s=${s//\\/\\\\}
    s=${s//\"/\\\"}
    printf '"%s"' "$s"
}

sql_name_list() { # sql_name_list <file of file names> -> quoted IN-list
    local list="" name
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        list+="$(sql_quote "$name"),"
    done < "$1"
    printf '%s' "${list%,}"
}

# Prints the destination coordinates of everything just imported, so the URL
# that was pasted in has an equivalent on the other side. file_name, guild,
# channel, year and month survive the import unchanged; session ids do not.
report_destination() { # report_destination <tmpdir> <base-url>
    local tmp=$1 base=$2 names sessions="" remapped
    names=$(sql_name_list "$tmp/new-recordings.list")
    [ -n "$names" ] || return 0
    : > "$tmp/dest-sessions.tsv"
    log "open in $base:"
    while IFS=$'\t' read -r guild channel year month name session; do
        [ -n "$name" ] || continue
        printf '        %s/dashboard/%s/audio/%s/%s/%s/%s\n' \
            "$base" "$guild" "$channel" "$year" "$month" "$name"
        if [ -n "$session" ]; then
            sessions+="$guild/$session"$'\n'
            printf '%s\t%s\n' "$name" "$session" >> "$tmp/dest-sessions.tsv"
        fi
    done < <(dest_tsv "SELECT guild_id, channel_id, year, month, file_name,
                             COALESCE(recording_session_id::text, '')
                        FROM audio_files
                       WHERE file_name IN ($names)
                       ORDER BY start_ts, file_name")
    while IFS=/ read -r guild session; do
        [ -n "$session" ] || continue
        printf '        %s/dashboard/%s/audio/session/%s\n' "$base" "$guild" "$session"
    done < <(printf '%s' "$sessions" | sort -u)

    # A session keeps its source id unless the destination already had one, so
    # say which ones moved instead of leaving the URL above to be puzzled over.
    [ -s "$tmp/source-sessions.tsv" ] || return 0
    remapped=$(awk -F'\t' 'NR == FNR { src[$1] = $2; next }
                           ($1 in src) && src[$1] != $2 { print src[$1] "\t" $2 }' \
        "$tmp/source-sessions.tsv" "$tmp/dest-sessions.tsv" | sort -u)
    [ -n "$remapped" ] || return 0
    while IFS=$'\t' read -r from to; do
        if [ -n "$to" ]; then log "session $from was already taken there; imported as $to"; fi
    done <<< "$remapped"
}

remote_sh() { # remote_sh <command> — quiet, exit status only
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        bash -lc "$1" >/dev/null 2>&1
    else
        # The quoted command is intentionally evaluated on the VPS.
        # shellcheck disable=SC2029
        ssh -n "$SAKIOT_DEV_SSH" "$1" >/dev/null 2>&1
    fi
}

remote_capture() { # remote_capture <command> — stdout, empty on failure
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        bash -lc "$1" 2>/dev/null
    else
        # shellcheck disable=SC2029
        ssh -n "$SAKIOT_DEV_SSH" "$1" 2>/dev/null
    fi
}

# Staging is reached through dev_login, which mints a token for DEV_ACCOUNT_ID
# rather than a Discord identity, so that is the account the permission checks
# run against and the one that needs guild access.
staging_dev_account_id() {
    if [ -n "${SAKIOT_DEV_STAGING_ACCOUNT_ID:-}" ]; then
        printf '%s' "$SAKIOT_DEV_STAGING_ACCOUNT_ID"
        return
    fi
    local env_file read_env id
    env_file=${SAKIOT_DEV_STAGING_ENV_FILE:-/etc/sakiot/staging.env}
    read_env="set -a; . $(sh_quote "$env_file"); set +a; printf %s \"\${DEV_ACCOUNT_ID:-}\""
    id=$(remote_capture "$read_env")
    [ -n "$id" ] || id=$(remote_capture "sudo -n -u sakiot /bin/bash -lc $(sh_quote "$read_env")")
    printf '%s' "$id"
}

# The staging data dir is 0750 sakiot:sakiot, so a developer in the sakiot group
# can read it but not write it, and ops/sudoers only grants passwordless sudo for
# the deploy wrapper. Pick the mechanism up front rather than discovering it
# halfway through, so nothing is written twice.
staging_write_mode() { # staging_write_mode <remote data dir>
    remote_sh "test -w $(sh_quote "$1")" && { printf direct; return; }
    remote_sh "sudo -n -u sakiot true" && { printf sudo_n; return; }
    printf sudo_tty
}

# Media and manifest go through one elevated step so sudo asks at most once.
publish_to_staging() { # publish_to_staging <tmpdir> <dest data dir> <manifest fragment>
    local tmp=$1 dest=$2 fragment=$3 mode
    local manifest_path="$dest/$STAGING_IMPORT_MANIFEST"
    local sudo_rsync=${SAKIOT_DEV_STAGING_RSYNC_PATH:-sudo -n -u sakiot rsync}
    mode=$(staging_write_mode "$dest")

    if [ "$mode" = direct ]; then
        log "installing media into $dest"
        if publish_direct "$tmp" "$dest" "$fragment" "$manifest_path"; then
            return
        fi
        # Writable at the top level but not all the way down; escalate before
        # the manifest is touched, so nothing gets appended twice.
        log "direct write failed part-way; elevating to the sakiot user"
        mode=$(remote_sh "sudo -n -u sakiot true" && printf sudo_n || printf sudo_tty)
    fi

    if [ "$mode" = sudo_n ]; then
        log "installing media into $dest as sakiot (passwordless sudo)"
        if [ "$SAKIOT_DEV_SSH" = local ]; then
            # shellcheck disable=SC2086
            $sudo_rsync -a --info=stats1 "$tmp/media/" "$dest/" || die "rsync into $dest failed"
        else
            rsync -a --info=stats1 --rsync-path="$sudo_rsync" \
                "$tmp/media/" "$SAKIOT_DEV_SSH:$dest/" || die "rsync into $dest failed"
        fi
        elevated_append "$fragment" "$manifest_path" -n \
            || die "media landed in $dest but appending to $manifest_path failed"
        return
    fi

    publish_via_stage "$tmp" "$dest" "$fragment" "$manifest_path"
}

publish_direct() { # publish_direct <tmpdir> <dest> <fragment> <manifest path>
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        rsync -a --info=stats1 "$1/media/" "$2/" || return 1
        cat "$3" >> "$4"
    else
        rsync -a --info=stats1 "$1/media/" "$SAKIOT_DEV_SSH:$2/" || return 1
        # shellcheck disable=SC2029
        ssh "$SAKIOT_DEV_SSH" "cat >> $(sh_quote "$4")" < "$3"
    fi
}

elevated_append() { # elevated_append <fragment> <manifest path> [-n]
    local cmd
    cmd="cat >> $(sh_quote "$2")"
    if [ "$SAKIOT_DEV_SSH" = local ]; then
        # Reading the fragment as the caller and appending as sakiot is the point.
        # shellcheck disable=SC2024
        sudo ${3:-} -u sakiot /bin/bash -lc "$cmd" < "$1"
    else
        # shellcheck disable=SC2029
        ssh "$SAKIOT_DEV_SSH" "sudo ${3:-} -u sakiot /bin/bash -lc $(sh_quote "$cmd")" < "$1"
    fi
}

# Last resort: neither group write nor passwordless sudo, so one step has to run
# as sakiot behind a password prompt. The payload is staged with a ready-made
# installer so the privileged command stays a single short line, which keeps the
# `ssh -t` interactive path simple and gives the caller something copy-pasteable
# when there is no terminal to prompt on.
publish_via_stage() { # publish_via_stage <tmpdir> <dest> <fragment> <manifest path>
    local tmp=$1 dest=$2 fragment=$3 manifest_path=$4 stage
    log "no group write and no passwordless sudo for $dest; staging the payload"

    # A staged installer keeps the command the caller has to copy free of any
    # nested quoting.
    cat > "$tmp/install.sh" <<EOF
#!/bin/sh
set -e
rsync -a --exclude .import-manifest --exclude install.sh "\$(dirname "\$0")/" $(sh_quote "$dest")/
cat "\$(dirname "\$0")/.import-manifest" >> $(sh_quote "$manifest_path")
EOF

    if [ "$SAKIOT_DEV_SSH" = local ]; then
        stage=$(mktemp -d /tmp/sakiot-dev-import.XXXXXX)
        chmod 0755 "$stage"
        cp -a "$tmp/media/." "$stage/"
        cp "$fragment" "$stage/.import-manifest"
        cp "$tmp/install.sh" "$stage/install.sh"
        chmod -R a+rX "$stage"
    else
        stage=$(ssh -n "$SAKIOT_DEV_SSH" \
            'd=$(mktemp -d /tmp/sakiot-dev-import.XXXXXX) && chmod 0755 "$d" && printf %s "$d"') \
            || die "could not create a staging directory on $SAKIOT_DEV_SSH"
        rsync -a --info=stats1 --chmod=D755,F644 \
            "$tmp/media/" "$SAKIOT_DEV_SSH:$stage/" || die "could not stage media on $SAKIOT_DEV_SSH"
        rsync -a --chmod=F644 "$fragment" "$SAKIOT_DEV_SSH:$stage/.import-manifest" \
            || die "could not stage the import manifest on $SAKIOT_DEV_SSH"
        rsync -a --chmod=F755 "$tmp/install.sh" "$SAKIOT_DEV_SSH:$stage/install.sh" \
            || die "could not stage the installer on $SAKIOT_DEV_SSH"
    fi

    # Wrapped for a remote shell, bare when the command runs here.
    local install_cmd="sudo -u sakiot sh $stage/install.sh && rm -rf $stage"
    local relax_cmd="sudo chmod -R g+rwX $dest && sudo find $dest -type d -exec chmod g+s {} +"
    if [ "$SAKIOT_DEV_SSH" != local ]; then
        install_cmd="ssh -t $SAKIOT_DEV_SSH '$install_cmd'"
        relax_cmd="ssh -t $SAKIOT_DEV_SSH '$relax_cmd'"
    fi

    if [ -t 0 ] && [ -t 1 ]; then
        log "installing as sakiot — sudo will ask for your password once"
        local status=0
        if [ "$SAKIOT_DEV_SSH" = local ]; then
            sudo -u sakiot sh "$stage/install.sh" || status=$?
            rm -rf "$stage"
        else
            # -t so sudo can prompt on the caller's terminal.
            # shellcheck disable=SC2029
            ssh -t "$SAKIOT_DEV_SSH" "sudo -u sakiot sh $(sh_quote "$stage/install.sh")" || status=$?
            # shellcheck disable=SC2029
            ssh -n "$SAKIOT_DEV_SSH" "rm -rf $(sh_quote "$stage")" >/dev/null 2>&1 || true
        fi
        if [ "$status" -eq 0 ]; then
            log "installed media into $dest as sakiot"
            log "tip: '$relax_cmd' once, and no later fetch needs sudo at all"
            return
        fi
        log "the elevated install did not complete (exit $status)"
    fi

    printf '\n'
    echo "  Media is staged at $stage but not installed: that step needs sudo on a"
    echo "  terminal. Run it yourself:"
    printf '\n'
    echo "    $install_cmd"
    printf '\n'
    echo "  Or remove the need for sudo permanently — you are already in the sakiot"
    echo "  group, so making the data dir group-writable once means every later fetch"
    echo "  installs directly:"
    printf '\n'
    echo "    $relax_cmd"
    printf '\n'
    die "database rows are imported and idempotent; only the media install is pending.
      Run one of the commands above, then re-run this fetch to install and verify."
}

confirm_staging_write() { # confirm_staging_write <recording count> <staging data dir>
    printf '\n'
    printf '\033[1;33m  ############################################################\033[0m\n'
    printf '\033[1;33m  #  COPYING %-4s PRODUCTION RECORDING(S) INTO STAGING        #\033[0m\n' "$1"
    printf '\033[1;33m  ############################################################\033[0m\n'
    echo "  target database : ${SAKIOT_DEV_STAGING_DB:-sakiot_staging} on $SAKIOT_DEV_SSH"
    echo "  target media dir: $2"
    echo "  Every imported session gets an 'imported_from_production' event at"
    echo "  offset 0 of its staging timeline, and each recording is appended to"
    echo "  $2/$STAGING_IMPORT_MANIFEST."
    echo "  A session keeps its production id unless staging already uses it, in"
    echo "  which case the run says which id it landed on instead."
    printf '\n'
    [ "$ASSUME_YES" -eq 1 ] && { log "proceeding (--yes)"; return; }
    [ -t 0 ] || die "refusing to write to staging without a terminal; pass --yes"
    printf 'Continue? [y/N] '
    local yn
    read -r yn
    case "$yn" in
        [Yy]*) ;;
        *) die "aborted" ;;
    esac
}

install_into_local() { # install_into_local <tmpdir>
    local tmp=$1 data_dir manifest recordings_manifest dev_id
    data_dir=$(env_get SAKIOT_DATA_DIR "$ROOT/data")
    manifest="$data_dir/.dev-fixtures.list"
    recordings_manifest="$data_dir/.dev-fixture-recordings.list"

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
    import_lookup_rows "$tmp"
    import_selection "$tmp"
    log "  recordings: $(wc -l < "$tmp/audio_files.tsv") row(s) from $SOURCE_LABEL"

    dev_id=$(env_get DEV_ACCOUNT_ID "$DEFAULT_DEV_ACCOUNT_ID")
    psql_local >/dev/null <<SQL
INSERT INTO guilds_present (guild_id)
    SELECT id FROM guilds WHERE id IN ($GUILD_IDS) ON CONFLICT DO NOTHING;
INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
    SELECT id, $dev_id, 'Fixture guild ' || id, NULL, true, 8, '{}'
    FROM guilds WHERE id IN ($GUILD_IDS) ON CONFLICT DO NOTHING;
SQL

    log "installing media files into $data_dir"
    rsync -a "$tmp/media/" "$data_dir/"

    if [ -n "$SELECTOR_KIND" ]; then
        # A targeted pull adds to the fixture set instead of replacing it, so
        # the recordings already fetched for other work survive.
        sort -u "$tmp/old-files.list" "$tmp/new-files.list" > "$tmp/manifest.list"
        sort -u "$tmp/old-recordings.list" "$tmp/new-recordings.list" > "$tmp/recordings.list"
    else
        prune_managed_fixtures "$tmp/new-recordings.list" "$tmp/old-recordings.list"
        while IFS= read -r f; do
            rm -f "$data_dir/$f"
        done < <(comm -23 "$tmp/old-files.list" "$tmp/new-files.list")
        cp "$tmp/new-files.list" "$tmp/manifest.list"
        cp "$tmp/new-recordings.list" "$tmp/recordings.list"
    fi
    find "$data_dir" -depth -type d -empty -delete 2>/dev/null || true
    mv "$tmp/manifest.list" "$manifest"
    mv "$tmp/recordings.list" "$recordings_manifest"
}

install_into_staging() { # install_into_staging <tmpdir>
    local tmp=$1 staging_data marker stamp who
    staging_data=${SAKIOT_DEV_STAGING_DATA:-/var/lib/sakiot-staging/data}
    confirm_staging_write "$(wc -l < "$tmp/audio_files.tsv")" "$staging_data"

    stamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    who="$(id -un)@$(hostname -s 2>/dev/null || hostname)"
    marker=$(printf '{"source":"production","origin_url":%s,"imported_at":%s,"imported_by":%s,"tool":"scripts/dev.sh"}' \
        "$(json_string "$SELECTOR_INPUT")" "$(json_string "$stamp")" "$(json_string "$who")")

    log "importing rows into staging"
    import_lookup_rows "$tmp"
    import_selection "$tmp" "$marker"
    log "  recordings: $(wc -l < "$tmp/audio_files.tsv") row(s) from production"

    STAGING_DEV_ACCOUNT_ID=$(staging_dev_account_id)
    if is_uint "$STAGING_DEV_ACCOUNT_ID" && [ "$STAGING_DEV_ACCOUNT_ID" != 0 ]; then
        # visible_channels_for_user() rejects a guild the viewer has no row for,
        # and dev_login never writes one. owner = true short-circuits the channel
        # permission maths, which the imported guild has no data for anyway.
        psql_dest >/dev/null <<SQL
INSERT INTO guilds_present (guild_id)
    SELECT id FROM guilds WHERE id IN ($GUILD_IDS) ON CONFLICT DO NOTHING;
INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
    SELECT id, $STAGING_DEV_ACCOUNT_ID, 'Imported from production ' || id, NULL, true, 8, '{}'
    FROM guilds WHERE id IN ($GUILD_IDS) ON CONFLICT DO NOTHING;
SQL
        log "granted the dev_login account $STAGING_DEV_ACCOUNT_ID access to guild $GUILD_IDS"
        log "  (listed as 'Imported from production $GUILD_IDS' in the guild picker)"
    else
        psql_dest >/dev/null <<SQL
INSERT INTO guilds_present (guild_id)
    SELECT id FROM guilds WHERE id IN ($GUILD_IDS) ON CONFLICT DO NOTHING;
SQL
        log "WARNING: could not read DEV_ACCOUNT_ID from the staging env file, so the"
        log "         dev_login account was not granted access to guild $GUILD_IDS."
        log "         Set SAKIOT_DEV_STAGING_ACCOUNT_ID and re-run, or insert it by hand."
    fi
    log "note: the staging bot rewrites guilds_present on startup and is not in this"
    log "      guild, so the entry disappears on its next restart — re-run to restore"

    awk -v stamp="$stamp" -v who="$who" -v url="$SELECTOR_INPUT" -F'\t' \
        '{ printf "%s\tfile_name=%s\tguild=%s\tby=%s\tfrom=%s\n", stamp, $1, $2, who, url }' \
        "$tmp/audio_files.tsv" > "$tmp/import-manifest.txt"
    publish_to_staging "$tmp" "$staging_data" "$tmp/import-manifest.txt"
    log "recorded the import in $staging_data/$STAGING_IMPORT_MANIFEST"
    verify_staging_visibility "$tmp" "$staging_data"
}

# Rows and files landing is not the same as the recording being reachable: the
# listing is gated on permissions.rs, which needs a membership row and a
# permission baseline that a data import does not create. Check the exact
# preconditions rather than claiming success.
verify_staging_visibility() { # verify_staging_visibility <tmpdir> <dest data dir>
    local tmp=$1 dest=$2 rel check missing count
    log "checking that staging can actually serve it:"

    # One round trip for the whole list rather than an ssh per file: each remote
    # call is a fresh connection, and an ssh inside a read loop reads the loop's
    # own stdin out from under it.
    check=$(while IFS= read -r rel; do
        printf 'test -f %s || printf %%s\\\\n %s\n' \
            "$(sh_quote "$dest/$rel")" "$(sh_quote "$rel")"
    done < "$tmp/new-files.list")
    missing=$(remote_capture "$check")
    if [ -z "$missing" ]; then
        log "    ok    $(wc -l < "$tmp/new-files.list") media file(s) in place under $dest"
    else
        while IFS= read -r rel; do
            if [ -n "$rel" ]; then log "    FAIL  not on disk: $rel"; fi
        done <<< "$missing"
    fi

    count=$(dest_tsv "SELECT count(*) FROM roles WHERE guild_id IN ($GUILD_IDS) AND role_id = guild_id")
    if [ "${count:-0}" -gt 0 ]; then
        log "    ok    @everyone role present (the permission baseline)"
    else
        log "    FAIL  no @everyone role for guild $GUILD_IDS; permission lookups error out"
    fi

    count=$(dest_tsv "SELECT count(*) FROM channels WHERE guild_id IN ($GUILD_IDS) AND type = 2")
    if [ "${count:-0}" -gt 0 ]; then
        log "    ok    $count voice channel row(s) for the guild"
    else
        log "    FAIL  no voice channel rows; the recordings tree stays empty"
    fi

    count=$(dest_tsv "SELECT count(*) FROM guilds_present WHERE guild_id IN ($GUILD_IDS)")
    if [ "${count:-0}" -gt 0 ]; then
        log "    ok    guild listed in guilds_present"
    else
        log "    FAIL  guild dropped from guilds_present (staging bot restarted); re-run this fetch"
    fi

    if is_uint "${STAGING_DEV_ACCOUNT_ID:-}" && [ "${STAGING_DEV_ACCOUNT_ID:-0}" != 0 ]; then
        count=$(dest_tsv "SELECT count(*) FROM user_guilds
                           WHERE id IN ($GUILD_IDS) AND user_id = $STAGING_DEV_ACCOUNT_ID")
        if [ "${count:-0}" -gt 0 ]; then
            log "    ok    dev_login account $STAGING_DEV_ACCOUNT_ID can see the guild"
        else
            log "    FAIL  dev_login account $STAGING_DEV_ACCOUNT_ID has no user_guilds row for"
            log "          guild $GUILD_IDS; visible_channels_for_user() returns 403 without one"
        fi
    else
        count=$(dest_tsv "SELECT count(*) FROM user_guilds WHERE id IN ($GUILD_IDS)")
        if [ "${count:-0}" -gt 0 ]; then
            log "    ok    $count account(s) hold a user_guilds row for the guild"
        else
            log "    FAIL  no account has a user_guilds row for guild $GUILD_IDS, so nothing"
            log "          can reach the recording. Set SAKIOT_DEV_STAGING_ACCOUNT_ID to the"
            log "          staging DEV_ACCOUNT_ID and re-run so it can be granted."
        fi
    fi
}

import_lookup_rows() { # import_lookup_rows <tmpdir>
    local tmp=$1
    import_tsv audio_file_finalize_reasons "$tmp/audio_file_finalize_reasons.tsv"
    import_tsv user_name_event_types "$tmp/user_name_event_types.tsv"
    import_tsv channel_type "$tmp/channel_type.tsv"
    import_tsv guilds "$tmp/guilds.tsv"
    import_tsv roles "$tmp/roles.tsv"
    import_tsv channels "$tmp/channels.tsv"
    import_tsv user_names "$tmp/user_names.tsv"
    import_tsv user_nicknames "$tmp/user_nicknames.tsv"
    import_tsv user_name_history "$tmp/user_name_history.tsv"
}

cmd_fetch_fixtures() (
    local count=20 count_given=0 guild_filter="" selector="" forced_kind="" source_arg=""
    SELECTOR_KIND=""
    SELECTOR_VALUE=""
    SELECTOR_HOST=""
    SELECTOR_INPUT=""
    DEST=local
    ASSUME_YES=0
    STAGING_IMPORT_MANIFEST=.imported-from-production.list
    while [ $# -gt 0 ]; do
        case "$1" in
            --count)
                [ $# -ge 2 ] || die "--count needs a value"
                is_uint "$2" || die "--count must be an unsigned integer"
                count=$2
                count_given=1
                shift 2
                ;;
            --guild)
                [ $# -ge 2 ] || die "--guild needs a value"
                is_uint "$2" || die "--guild must be an unsigned integer"
                guild_filter="AND guild_id = $2"
                shift 2
                ;;
            --url|--id)
                [ $# -ge 2 ] || die "$1 needs a value"
                selector=$2
                shift 2
                ;;
            --session)
                [ $# -ge 2 ] || die "--session needs a value"
                selector=$2
                forced_kind=session
                shift 2
                ;;
            --recording|--file)
                [ $# -ge 2 ] || die "$1 needs a value"
                selector=$2
                forced_kind=recording
                shift 2
                ;;
            --source)
                [ $# -ge 2 ] || die "--source needs a value"
                source_arg=$2
                shift 2
                ;;
            --into)
                [ $# -ge 2 ] || die "--into needs a value"
                DEST=$2
                shift 2
                ;;
            --yes|-y) ASSUME_YES=1; shift ;;
            -*) die "unknown flag: $1
      supported: --count N, --guild ID, --url/--id <url-or-id>, --session ID,
                 --recording ID|FILE_NAME, --source prod|staging, --into local|staging, --yes" ;;
            *)
                [ -z "$selector" ] || die "give only one recording selector (got '$selector' and '$1')"
                selector=$1
                shift
                ;;
        esac
    done

    case "$DEST" in
        local|staging) ;;
        *) die "unknown --into target: $DEST (supported: local, staging)" ;;
    esac

    if [ -n "$selector" ]; then
        [ "$count_given" -eq 0 ] || die "--count fetches recent recordings; it cannot be combined with a selector"
        [ -z "$guild_filter" ] || die "--guild filters bulk fetches only; a selector already names one recording"
        SELECTOR_INPUT=$selector
        case "$forced_kind" in
            session)
                is_uint "$selector" || die "--session needs a numeric recording_sessions.id"
                SELECTOR_KIND=session
                SELECTOR_VALUE=$selector
                ;;
            recording)
                parse_selector "$selector"
                [ "$SELECTOR_KIND" = numeric ] && SELECTOR_KIND=audio_id
                ;;
            *) parse_selector "$selector" ;;
        esac
    fi

    if [ -z "$source_arg" ] && [ -n "$SELECTOR_HOST" ]; then
        source_arg=$(source_for_host "$SELECTOR_HOST")
        [ -n "$source_arg" ] && log "$SELECTOR_HOST is the $source_arg frontend, reading from there"
    fi
    [ -n "$source_arg" ] || source_arg=${SAKIOT_DEV_SOURCE:-$(env_get SAKIOT_DEV_SOURCE)}
    [ -n "$source_arg" ] || source_arg=staging
    resolve_source "$source_arg"

    if [ "$DEST" = staging ]; then
        [ -n "$SELECTOR_KIND" ] \
            || die "--into staging needs one recording or session, not a bulk --count fetch"
        [ "$SOURCE" = prod ] \
            || die "--into staging copies production data into staging, but the source is $SOURCE_LABEL"
    fi

    if [ -z "${SAKIOT_DEV_SSH:-}" ]; then
        SAKIOT_DEV_SSH=$(env_get SAKIOT_DEV_SSH)
    fi
    if [ -z "${SAKIOT_DEV_SSH:-}" ] && [ -d "$REMOTE_DATA" ]; then
        log "$SOURCE_LABEL data found on this machine, using local mode (no SSH)"
        SAKIOT_DEV_SSH=local
    fi
    if [ -z "${SAKIOT_DEV_SSH:-}" ] && [ -t 0 ]; then
        printf 'VPS SSH target (user@host): '
        read -r SAKIOT_DEV_SSH
    fi
    : "${SAKIOT_DEV_SSH:?set SAKIOT_DEV_SSH=user@vps-host (personal SSH access to the VPS), or 'local' on the VPS itself}"

    [ "$DEST" = staging ] || [ -f "$ROOT/.env" ] || die "run 'scripts/dev.sh db' first"
    [ "$SAKIOT_DEV_SSH" = local ] || need ssh "for remote database export"
    need rsync "for file transfer"
    configure_remote_psql
    configure_dest_psql

    local tmp
    tmp=$(mktemp -d)
    trap 'rm -rf "${tmp:-}"' EXIT

    resolve_numeric_selector
    if [ -n "$SELECTOR_KIND" ]; then
        log "exporting $SELECTOR_KIND $SELECTOR_VALUE from $REMOTE_DB ($SOURCE_LABEL) on $SAKIOT_DEV_SSH"
        fetch_tsv "SELECT id FROM audio_files WHERE $(selector_predicate) ORDER BY id" \
            > "$tmp/audio_file_ids.tsv"
        [ -s "$tmp/audio_file_ids.tsv" ] \
            || die "no recording in $REMOTE_DB ($SOURCE_LABEL) matches $SELECTOR_KIND $SELECTOR_VALUE"
    else
        log "exporting up to $count recent recordings from $REMOTE_DB ($SOURCE_LABEL) on $SAKIOT_DEV_SSH"
        fetch_tsv "SELECT id FROM audio_files WHERE reaped = false AND end_ts IS NOT NULL $guild_filter ORDER BY id DESC LIMIT $count" \
            > "$tmp/audio_file_ids.tsv"
        [ -s "$tmp/audio_file_ids.tsv" ] || die "$SOURCE_LABEL has no matching recordings — nothing to fetch"
    fi
    local audio_file_ids
    audio_file_ids=$(sort -un "$tmp/audio_file_ids.tsv" | paste -sd, -)
    fetch_tsv "SELECT * FROM audio_files WHERE id IN ($audio_file_ids) ORDER BY id DESC" > "$tmp/audio_files.tsv"

    # Archive-pruned originals must exist locally before rsync. Targeted
    # restore full-hash verifies existing files and atomically downloads only
    # missing/mismatched selected recordings.
    hydrate_remote_recordings "$audio_file_ids"

    # The logical parent of every fragment, so the audio_files FK resolves in the
    # destination and the session player has its gaps and timeline.
    local session_ids
    session_ids=$(fetch_tsv "SELECT DISTINCT recording_session_id FROM audio_files
                              WHERE id IN ($audio_file_ids) AND recording_session_id IS NOT NULL" \
        | sort -un | paste -sd, -)
    : > "$tmp/recording_sessions.tsv"
    : > "$tmp/recording_gaps.tsv"
    : > "$tmp/recording_session_events.tsv"
    # Which session each fragment came from, so a session id that could not be
    # claimed in the destination can be named rather than inferred.
    fetch_tsv "SELECT file_name, recording_session_id FROM audio_files
                WHERE id IN ($audio_file_ids) AND recording_session_id IS NOT NULL" \
        > "$tmp/source-sessions.tsv"
    if [ -n "$session_ids" ]; then
        fetch_tsv "SELECT * FROM recording_sessions WHERE id IN ($session_ids)" > "$tmp/recording_sessions.tsv"
        fetch_tsv "SELECT * FROM recording_gaps WHERE recording_session_id IN ($session_ids)" > "$tmp/recording_gaps.tsv"
        fetch_tsv "SELECT * FROM recording_session_events WHERE recording_session_id IN ($session_ids)" > "$tmp/recording_session_events.tsv"
    fi

    # audio_files columns: file_name guild_id channel_id user_id year month ...
    local channel_ids user_ids
    GUILD_IDS=$(cut -f2 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)
    channel_ids=$(cut -f3 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)
    user_ids=$(cut -f4 "$tmp/audio_files.tsv" | sort -un | paste -sd, -)

    # Voice events are keyed by guild and wall-clock time, not by recording, so
    # scope them to the span the imported fragments and sessions cover. Without
    # them every timeline lane except "recording" is empty, and the markers on
    # the single-recording view never appear.
    # The margin matters: the connect operation that starts a session completes
    # a moment before the first fragment, so an exact-window fetch drops exactly
    # the events worth having when a recording misbehaved.
    local span_sql span_margin=${SAKIOT_DEV_EVENT_MARGIN:-5 minutes}
    span_sql="SELECT MIN(lo) - interval $(sql_quote "$span_margin") AS lo,
                     MAX(hi) + interval $(sql_quote "$span_margin") AS hi FROM (
        SELECT to_timestamp(COALESCE(af.start_ts, 0) / 1000.0),
               to_timestamp(COALESCE(af.end_ts, af.start_ts, 0) / 1000.0)
          FROM audio_files af WHERE af.id IN ($audio_file_ids)
        UNION ALL
        SELECT rs.started_at, COALESCE(rs.ended_at, rs.started_at)
          FROM recording_sessions rs WHERE rs.id IN (${session_ids:--1})
    ) s(lo, hi)"
    fetch_tsv "SELECT * FROM voice_state_event_types" > "$tmp/voice_state_event_types.tsv"
    # Every user in the channel shows up on the timeline, not just the recorded
    # one, so this is deliberately not filtered by user_id.
    fetch_tsv "WITH span AS ($span_sql)
               SELECT v.* FROM voice_state_events v, span
                WHERE v.guild_id IN ($GUILD_IDS)
                  AND v.occurred_at >= span.lo AND v.occurred_at <= span.hi
                ORDER BY v.occurred_at, v.id" > "$tmp/voice_state_events.tsv"
    fetch_tsv "WITH span AS ($span_sql)
               SELECT e.* FROM voice_connection_events e, span
                WHERE e.guild_id IN ($GUILD_IDS)
                  AND e.completed_at >= span.lo AND e.started_at <= span.hi
                ORDER BY e.completed_at, e.id" > "$tmp/voice_connection_events.tsv"

    # voice_state_events columns: id guild_id channel_id user_id event_type_id ...
    # Pull names and channels for the bystanders those events reference too.
    user_ids=$( { cut -f4 "$tmp/audio_files.tsv"; cut -f4 "$tmp/voice_state_events.tsv"; } \
        | grep -E '^[0-9]+$' | sort -un | paste -sd, -)
    channel_ids=$( { cut -f3 "$tmp/audio_files.tsv"; cut -f3 "$tmp/voice_state_events.tsv"; } \
        | grep -E '^[0-9]+$' | sort -un | paste -sd, -)

    fetch_tsv "SELECT * FROM guilds WHERE id IN ($GUILD_IDS)" > "$tmp/guilds.tsv"
    fetch_tsv "SELECT * FROM roles WHERE guild_id IN ($GUILD_IDS)" > "$tmp/roles.tsv"
    fetch_tsv "SELECT * FROM channels WHERE channel_id IN ($channel_ids)" > "$tmp/channels.tsv"
    fetch_tsv "SELECT * FROM user_names WHERE user_id IN ($user_ids)" > "$tmp/user_names.tsv"
    fetch_tsv "SELECT * FROM user_nicknames WHERE user_id IN ($user_ids) AND guild_id IN ($GUILD_IDS)" > "$tmp/user_nicknames.tsv"
    fetch_tsv "SELECT * FROM user_name_history WHERE user_id IN ($user_ids)" > "$tmp/user_name_history.tsv"
    # small lookup tables, populated at runtime on the source deployment
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

    # Download completely before changing anything in the destination.
    local src
    mkdir -p "$tmp/media"
    src="$SAKIOT_DEV_SSH:$REMOTE_DATA/"
    [ "$SAKIOT_DEV_SSH" = local ] && src="$REMOTE_DATA/"
    log "downloading media files from $SOURCE_LABEL"
    rsync -a --info=stats1 --ignore-missing-args \
        --files-from="$tmp/files.list" "$src" "$tmp/media/"

    # `if`, not `&&`: a false `&&` makes the loop exit non-zero, and with
    # `set -euo pipefail` that silently ends the run mid-fetch.
    while IFS= read -r f; do
        if [ -f "$tmp/media/$f" ]; then printf '%s\n' "$f"; fi
    done < "$tmp/files.list" | sort -u > "$tmp/new-files.list"

    if [ "$DEST" = staging ]; then
        install_into_staging "$tmp"
        report_destination "$tmp" "${SAKIOT_DEV_STAGING_URL:-https://staging.patrykstyla.com}"
        log "done: $(wc -l < "$tmp/audio_files.tsv") production recording(s) copied into staging and marked as imported"
    else
        ensure_data_dirs
        install_into_local "$tmp"
        report_destination "$tmp" "${SAKIOT_DEV_LOCAL_URL:-http://localhost:8081}"
        log "done: $(wc -l < "$tmp/audio_files.tsv") recording(s) from $SOURCE_LABEL guild(s) $GUILD_IDS"
    fi
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
    fetch)
        shift
        [ $# -gt 0 ] || die "fetch needs a dashboard URL or an id (see 'scripts/dev.sh help')"
        cmd_fetch_fixtures "$@"
        ;;
    clean) cmd_clean ;;
    help|-h|--help) awk 'NR > 1 && /^#/ { print; next } NR > 1 { exit }' "$0" ;;
    *) die "unknown command: $1 (up|db|down|reset|fetch-fixtures|fetch|clean)" ;;
esac

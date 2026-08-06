# Sakiot

Sakiot is a Discord voice-recording system. The bot records voice activity,
stores audio and metadata, and exposes recordings through a web application.

## Repository Layout

- `fbi-agent` - Discord bot and voice recorder.
- `web-server` - HTTP API, authentication, media serving, and gRPC client.
- `sakiot-stage` - React frontend consuming the HTTP API.
- `sakiot-paths` - Shared Rust crate for filesystem and URL conventions.
- `sakiot-proto` - Shared gRPC contract and generated Rust types.
- `sakiot-db` - Canonical PostgreSQL migrations and backup tooling.
- `data` - Local runtime media. Ignored by Git.

The Rust services share `sakiot-paths`, `sakiot-proto`, and one database schema.
Changes spanning these contracts can therefore be committed atomically.

## Local Development

One command brings up a local debug environment for `web-server` on any
machine (Docker, `sqlx-cli`, FFmpeg—including `ffprobe`—and `audiowaveform`
are required):

```sh
scripts/dev.sh
```

On first run it generates a root `.env` with local values (random JWT and dev
login secrets, Postgres on `localhost:54320`) and a `.env.development.local`
that points the frontend at the local API. It then starts Postgres via
`compose.dev.yml`, runs migrations, seeds a dev account and guild, asks how many
staging recordings, clips and stamps to copy (`0` skips each), and starts
`web-server` under `cargo watch` with the `dev-login` feature, so saving a Rust
file rebuilds and restarts the server. Discord OAuth is not needed:
the frontend's dev login button calls `/api/dev_login` using
`VITE_DEV_LOGIN_SECRET`.

Stopping the script with Ctrl+C also stops the local PostgreSQL container. Its
named volume is preserved, so the next run starts with the same database.

Run the frontend against it in another terminal:

```sh
cd sakiot-stage
bun dev
```

Other subcommands:

```sh
scripts/dev.sh db              # only start Postgres + migrate + seed
scripts/dev.sh down            # stop Postgres
scripts/dev.sh reset           # drop the local database volume and re-seed
scripts/dev.sh fetch-fixtures  # copy recordings/clips/stamps from staging (see below)
scripts/dev.sh fetch <what>    # copy one recording/session/clip/stamp (see below)
scripts/dev.sh clean           # drop the db volume + delete fetched fixture files
```

The synthetic seed leaves the recordings list empty. To test the recordings
UI with real audio, waveforms, and metadata, pull a sample from a deployed
instance over your personal SSH access (read-only on the VPS side; nothing
is committed to the repository). Set `SAKIOT_DEV_SSH` in the git-ignored `.env`
to avoid entering the SSH target when the startup prompt requests recordings:

```sh
SAKIOT_DEV_SSH=user@vps-host
# Or invoke the standalone command directly:
SAKIOT_DEV_SSH=user@vps-host scripts/dev.sh fetch-fixtures --count 20 --clips 5 --stamps 5
# optionally: --guild <id>
```

Recordings are taken newest-first, because a recent one is usually what is being
debugged. Clips and stamps accumulate over the whole history and the interesting
ones are as likely to be old as new, so `--clips` and `--stamps` take a random
sample instead; the run says how large the pool was:

```
[dev] sampling 5 of 412 clip(s) in staging at random
```

A positive `--count` replaces the previously managed fixture recordings and
media; it does not touch unrelated local data. It is the only selection that
re-answers "which recordings do I want?", so it is the only one that replaces
anything: `--count 0` keeps the existing set even when the same run asks for
clips or stamps, and `--clips`/`--stamps` only ever add. Tracking rows in
`media_objects` for a replaced recording are dropped with it — the FK is
`ON DELETE RESTRICT`, and the local server re-reconciles what it still has on
its next pass. The remote export and media download complete before replacement
begins, so a remote failure leaves the previous set intact.

### Pulling one specific recording, clip or stamp

When a particular production file misbehaves or makes a good test sample, hand
its dashboard URL straight to `fetch`:

```sh
scripts/dev.sh fetch https://patrykstyla.com/dashboard/362257054829641758/audio/763782256980131892/2026/7/1785336946781-161172393719496704
scripts/dev.sh fetch https://patrykstyla.com/dashboard/362257054829641758/audio/session/384
scripts/dev.sh fetch https://patrykstyla.com/dashboard/362257054829641758/clips/2ae61e36-ada8-45df-baed-8aa26386c826
scripts/dev.sh fetch 1785336946781-161172393719496704   # bare file name
scripts/dev.sh fetch 2ae61e36-ada8-45df-baed-8aa26386c826  # bare clip UUID
scripts/dev.sh fetch 384                                # bare id, resolved remotely
scripts/dev.sh fetch --stamp 1204                       # stamps have no URL of their own
```

A file URL pulls that fragment; a session URL pulls every fragment of the
logical session. Both also pull the parent `recording_sessions` row plus its
gaps and timeline events. Unlike `--count`, this **adds** to the fixture set
instead of replacing it, and re-running the same fetch is a no-op.

### Clips and stamps travel with their session

A clip or a stamp is an annotation on a recording session, so neither means
anything on its own and the selection is widened in both directions:

- naming a clip or a stamp pulls the session behind it **whole** — every
  fragment, not just the one the annotation happens to sit on. Both listings are
  gated on the viewer being allowed to see *every* fragment of the session, so a
  partial import would land rows nothing can display;
- naming a recording or a session pulls the clips and stamps hanging off it, so
  an imported session looks the way it does in the source instead of arriving
  stripped of every annotation.

A pre-session clip names its source recording in `original_file_name` rather than
through `recording_session_id`, and that recording is pulled too. A session whose
fragments have all been reaped still comes across when a clip or stamp points at
it — it is the only thing the annotation hangs off.

Clip media (`clips/{YYYY}/{MM}/{uuid}.ogg`) is downloaded alongside the
recordings, and archive-pruned clips are materialized first, the same way
recordings are.

`clips.clip_id` is a UUID minted at capture time, so it survives the import
untouched: a `/clips/<uuid>` URL works verbatim against the destination, exactly
as a file URL does. Stamps have no stable key — `stamps.id` is a dense sequence
that means something different in every environment — so their ids are always
reissued, and a re-import recognizes a stamp it has already seen by what actually
identifies an occurrence: who stamped whom, in which channel, at which
`stamp_ts`. The stamp page is per guild (`/stamps/<guild_id>`), which is what the
run prints.

The audio event timeline draws from three tables, so all three come across.
`recording_session_events` feeds the "recording" lane; `voice_state_events` feeds
mute, deafen, suppress, channel and media; `voice_connection_events` feeds
"connection". The latter two are keyed by guild and wall-clock time rather than
by recording, so they are scoped to the span the imported fragments cover, plus
a margin (`SAKIOT_DEV_EVENT_MARGIN`, default 5 minutes) — the connect operation
that starts a session completes just before its first fragment and would
otherwise be missed. Events for every user in the channel are pulled, not just
the recorded one, along with their names.

The host in the URL picks the source, so a `patrykstyla.com` link reads from
production and a `staging.patrykstyla.com` one from staging. Override with
`--source prod|staging`, or set `SAKIOT_DEV_SOURCE` in `.env` for bare ids. A
bare number can be either an `audio_files.id` or a `recording_sessions.id`; the
script looks it up and asks for `--recording` or `--session` only if both match.
`stamps.id` is deliberately left out of that guess — it is a third dense sequence,
so including it would make almost every bare number ambiguous; name a stamp with
`--stamp` instead. A bare UUID can only be a clip, so it needs no disambiguation.

URLs stay portable. `file_name`, guild, channel, year and month survive the
import unchanged, and a session keeps its source `recording_sessions.id` when the
destination has that id free — so both URL shapes work verbatim, only the host
changes. If the destination already uses that id the session gets a fresh one
rather than displacing anything, and the run says so:

```
[dev] session 376 was already taken there; imported as 41
```

`audio_files.id` is always reissued: nothing addresses a recording by it, while
`media_objects` and `stamps` reference it, so preserving it would only risk
collisions. An imported stamp is repointed at the id its recording landed on, so
it stays attached to the fragment it was placed on rather than to whichever row
happens to hold that number in the destination. The command prints the URLs to
open when it finishes.

### Copying production data into staging

`--into staging` puts the same selection into the staging instance instead of
the local dev database, for reproducing something on `staging.patrykstyla.com`:

```sh
scripts/dev.sh fetch https://patrykstyla.com/dashboard/362.../audio/session/384 --into staging
scripts/dev.sh fetch https://patrykstyla.com/dashboard/362.../clips/2ae61e36-... --into staging
scripts/dev.sh fetch --stamp 1204 --source prod --into staging
```

It takes one named recording, session, clip or stamp — never a bulk or random
sample, which is not what reproducing a specific report needs.

It asks for confirmation first, and the copy is deliberately obvious afterwards:

- every imported session gets an `imported_from_production` event at offset 0 of
  its staging timeline, carrying the origin URL, the origin session id, who ran
  the import, and when — visible as a marker in the session player;
- each recording, clip and stamp is appended to
  `/var/lib/sakiot-staging/data/.imported-from-production.list` — a stamp by its
  production id and `stamp_ts`, since staging reissues the id;
- the run prints a banner naming the target database, the media directory, and
  how many recordings, clips and stamps are about to cross.

Importing the rows and files is not enough to make a recording *reachable*. The
listing goes through `visible_channels_for_user`, which returns 403 unless the
viewing account has a `user_guilds` row for that guild, and staging is reached
through `dev_login` — a token for `DEV_ACCOUNT_ID`, never a Discord identity, so
nothing populates that table on its own. The import therefore reads
`DEV_ACCOUNT_ID` out of `staging.env` and grants that account access to the
imported guild, listed as `Imported from production <id>` in the guild picker.
Override the account with `SAKIOT_DEV_STAGING_ACCOUNT_ID` if the env file is not
readable.

`guilds_present` is topped up for the same reason. The staging bot rewrites that
table on startup and is not a member of a production guild, so the entry
disappears on its next restart — re-run the fetch to restore it.

The command finishes with a pass/fail line per precondition — media files on
disk, the imported clip and stamp rows, the `@everyone` role, voice channel rows,
`guilds_present`, and the `dev_login` account's access — instead of reporting
success on data that nothing can serve.

### Remote access details

On the VPS itself the command detects the source data directory and switches to
local mode automatically — no `SAKIOT_DEV_SSH` needed. By default, the export
runs on the VPS using the source's env file (`/etc/sakiot/production.env` or
`/etc/sakiot/staging.env`) when readable, then tries `sudo -n -u postgres psql`,
then direct `psql`. For a nonstandard database setup, override the remote
command:

```sh
SAKIOT_DEV_REMOTE_PSQL="sudo -n -u postgres psql" scripts/dev.sh fetch-fixtures
```

Each candidate is accepted only if it actually holds `SELECT` on `audio_files`,
`recording_sessions`, `clips` and `stamps`, so a role that can connect but was
never granted the tables is rejected up front instead of failing mid-export.
Writing into staging is probed the same way, for `INSERT`.

Archive-pruned recordings and clips are materialized and SHA-256 verified before
`rsync` (`web_server media restore --audio-file-ids` / `--clip-ids`). The workflow
first tries the source environment/binary directly, then `sudo -n -u sakiot`.

Clips are checked on the source disk first and only the genuinely absent ones are
requested, so the archive is usually not involved at all. The two failures are
also weighted differently: a fetch whose *audio* never arrives is not worth
having, so a failed recording restore aborts, while a failed clip restore warns
and carries on — the rows and recordings are still useful, and the run ends by
naming each clip that imported without audio. That also keeps the script working
against a deployment older than `media restore --clip-ids`, which would otherwise
reject the call outright.

Override unusual layouts with
`SAKIOT_DEV_REMOTE_WEB_BINARY`, `SAKIOT_DEV_REMOTE_ENV_FILE`,
`SAKIOT_DEV_REMOTE_DB`, `SAKIOT_DEV_REMOTE_DATA`, or a complete
`SAKIOT_DEV_REMOTE_HYDRATE` command — which is invoked as
`$SAKIOT_DEV_REMOTE_HYDRATE --audio-file-ids <ids>` or
`--clip-ids <ids>`, so it must accept the selector flag. Writing into staging uses
`SAKIOT_DEV_STAGING_ENV_FILE`, `SAKIOT_DEV_STAGING_DB`,
`SAKIOT_DEV_STAGING_DATA`, `SAKIOT_DEV_STAGING_PSQL` and
`SAKIOT_DEV_STAGING_RSYNC_PATH`.

## Environment

Copy the root example once and fill in local credentials:

```sh
cp .env.example .env
chmod 600 .env
```

The root `.env` is used by both Rust services, SQLx macros and CLI commands, and
the database backup scripts. Set `SAKIOT_ENV_FILE` to override its path for
backup jobs. Frontend development values live in root `.env.development`; Vite
loads environment files from the monorepo root.

Backblaze archive variables default to disabled for local development. Enabled
deployments require every `SAKIOT_MEDIA_S3_*` value and fail startup on partial
configuration. Provisioning, migration, verification, restore, and rollback
steps live in [`ops/B2_MEDIA_ARCHIVE.md`](ops/B2_MEDIA_ARCHIVE.md).

Database integration tests use `SAKIOT_TEST_DATABASE_URL`. Export it as
`DATABASE_URL` when running tests; SQLx creates a disposable database per test:

```sh
set -a
. ./.env
set +a
DATABASE_URL="$SAKIOT_TEST_DATABASE_URL" cargo test --workspace
```

For isolated local PostgreSQL on port `54320`:

```sh
docker compose -f compose.dev.yml up -d
DATABASE_URL=postgres://postgres:password@localhost:54320/sakiot_rouvas \
  sqlx migrate run --source sakiot-db/migrations
```

## Rust Workspace

The repository root is a Cargo workspace containing both services and both
shared crates.

```sh
cargo build --workspace
DATABASE_URL="$SAKIOT_TEST_DATABASE_URL" cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

SQLx query metadata is checked into `.sqlx` so rust-analyzer and offline builds
do not need database credentials. After changing a `query!` macro or the
database schema, refresh it against a clean, disposable PostgreSQL container:

```sh
scripts/sqlx-prepare.sh
```

The script requires Docker and `sqlx-cli`. It does not use or modify the local
development or VPS database. The repository's pre-commit hook runs the same
command in check mode when staged migrations or Rust files containing SQLx
macros change.

Service-specific commands remain available:

```sh
cargo run -p fbi_agent
cargo run -p web_server
```

## Frontend

```sh
cd sakiot-stage
bun install
bun run test
bun run build
```

## Git Hooks

Enable the repository's checks once per clone:

```sh
git config core.hooksPath .githooks
```

The pre-commit hook checks SQLx metadata when relevant migrations or Rust files
change. The pre-push hook runs the same Rust and frontend formatting checks used
by CI. To fix formatting before committing:

```sh
cargo fmt --all
cd sakiot-stage
bun run format
```

Generate frontend API types directly from `web-server`'s compile-time OpenAPI
document; no running server or database is required:

```sh
cd sakiot-stage
bun run generate:api-types
bun run check:api-types
```

`check:api-types` exits nonzero when the checked-in types are stale. Set
`OPENAPI_URL` only when intentionally generating from another OpenAPI source.

## Database

Migrations have a single owner and are not run by either service:

```sh
cd sakiot-db
sqlx migrate info --source migrations
sqlx migrate run --source migrations
cd ..
scripts/sqlx-prepare.sh
```

Regenerate and commit `.sqlx` after every database migration so SQLx macros,
rust-analyzer, and offline builds validate queries against the current schema.

## Runtime Media

Media defaults to the repository's `data` directory. Override this with
`SAKIOT_DATA_DIR`, for example `/data` in containers with a shared volume.

Each component has its own README with configuration and deployment details.
Pushes to `main` auto-deploy to a staging instance; production ships on strict
`vX.Y.Z` tags (use `ops/release`). Staging, tag deployment, and rollback are
documented in `ops/README.md`.

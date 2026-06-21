# Sakiot

Sakiot is a Discord voice-recording system. The bot records voice activity,
stores audio and metadata, and exposes recordings through a web application.

## Repository Layout

- `FBI-agent` - Discord bot and voice recorder.
- `web_server` - HTTP API, authentication, media serving, and gRPC client.
- `sakiot_stage` - React frontend consuming the HTTP API.
- `sakiot-paths` - Shared Rust crate for filesystem and URL conventions.
- `sakiot-proto` - Shared gRPC contract and generated Rust types.
- `sakiot-db` - Canonical PostgreSQL migrations and backup tooling.
- `data` - Local runtime media. Ignored by Git.

The Rust services share `sakiot-paths`, `sakiot-proto`, and one database schema.
Changes spanning these contracts can therefore be committed atomically.

## Local Development

One command brings up a local debug environment for `web_server` on any
machine (Docker plus `sqlx-cli` required):

```sh
scripts/dev.sh
```

On first run it generates a root `.env` with local values (random JWT and dev
login secrets, Postgres on `localhost:54320`) and a `.env.development.local`
that points the frontend at the local API. It then starts Postgres via
`compose.dev.yml`, runs migrations, seeds a dev account and guild, asks how many
staging recordings to copy (`0` skips), and starts `web_server` under
`cargo watch` with the `dev-login` feature, so saving a Rust file rebuilds and
restarts the server. Discord OAuth is not needed:
the frontend's dev login button calls `/api/dev_login` using
`VITE_DEV_LOGIN_SECRET`.

Stopping the script with Ctrl+C also stops the local PostgreSQL container. Its
named volume is preserved, so the next run starts with the same database.

Run the frontend against it in another terminal:

```sh
cd sakiot_stage
bun dev
```

Other subcommands:

```sh
scripts/dev.sh db              # only start Postgres + migrate + seed
scripts/dev.sh down            # stop Postgres
scripts/dev.sh reset           # drop the local database volume and re-seed
scripts/dev.sh fetch-fixtures  # copy real recordings from staging (see below)
scripts/dev.sh clean           # drop the db volume + delete fetched fixture files
```

The synthetic seed leaves the recordings list empty. To test the recordings
UI with real audio, waveforms, and metadata, pull a sample from the staging
instance over your personal SSH access (read-only on the VPS side; nothing
is committed to the repository). Set `SAKIOT_DEV_SSH` in the git-ignored `.env`
to avoid entering the SSH target when the startup prompt requests recordings:

```sh
SAKIOT_DEV_SSH=user@vps-host
# Or invoke the standalone command directly:
SAKIOT_DEV_SSH=user@vps-host scripts/dev.sh fetch-fixtures --count 20
# optionally: --guild <id>
```

A positive count replaces the previously managed fixture recordings and media;
it does not touch unrelated local data. Entering `0` at startup keeps the
existing fixture set, whose recording count is shown in the prompt. The remote
export and media download complete before replacement begins, so a remote
failure leaves the previous set intact.

On the VPS itself fetch-fixtures detects `/var/lib/sakiot-staging/data` and
switches to local mode automatically — no `SAKIOT_DEV_SSH` needed. By default,
the export runs on the VPS using `/etc/sakiot/staging.env` when readable,
then tries `sudo -n -u postgres psql`, then direct `psql`. For a nonstandard
database setup, override the remote command:

```sh
SAKIOT_DEV_REMOTE_PSQL="sudo -n -u postgres psql" scripts/dev.sh fetch-fixtures
```

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
cd sakiot_stage
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
cd sakiot_stage
bun run format
```

Generate frontend API types while `web_server` is serving its OpenAPI document:

```sh
cd sakiot_stage
bun run generate:api-types
```

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

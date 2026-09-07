# Sakiot

Sakiot is a Discord voice-recording system. The bot records voice activity,
stores audio and metadata, and exposes recordings through a web application.

## Repository Layout

- `fbi-agent` - Discord bot and voice recorder.
- `web-server` - HTTP API, authentication, media serving, and gRPC client.
- `sakiot-stage` - React frontend consuming the HTTP API.
- `sakiot-paths` - Shared Rust crate for filesystem and URL conventions.
- `sakiot-proto` - Shared gRPC contract and generated Rust types.
- `sakiot-storage` - Shared Backblaze B2 archive configuration and S3 client.
- `sakiot-DSP` - Shared native/WASM audio processing; a standalone Cargo workspace.
- `sakiot-db` - Canonical PostgreSQL migrations and backup tooling.
- `ops/sakiot-dev` - Typed `cargo dev` local-development orchestration.
- `ops/sakiot-deploy` - Rust deployment engine.
- `data` - Local runtime media. Ignored by Git.

The Rust services share `sakiot-paths`, `sakiot-proto`, and one database schema.
Changes spanning these contracts can therefore be committed atomically.

## Local Development

One command brings up a local debug environment for `web-server` and the
frontend (Docker, FFmpeg—including `ffprobe` and the `rubberband` audio
filter—`audiowaveform`, Cargo Watch, Bun, and `rsync` for fixture transfers are
required):

```sh
cargo dev up
```

On first run it atomically generates a root `.env` with local values (random
JWT and dev-login secrets, Postgres on `localhost:54320`) and a
`.env.development.local` that points the frontend at the local API. It then
starts Postgres via `compose.dev.yml`, runs embedded migrations, seeds a dev
account and guild, and applies the requested fixture policy. The default
interactive prompt offers skip, full, or custom; non-interactive invocations
must choose `--fixtures skip` or `--fixtures full`.

`cargo dev up` supervises `web-server` under Cargo Watch and the frontend under
Bun, prefixes their output, and stops both when either exits or Ctrl+C is
pressed. Discord OAuth is not needed: the frontend's dev login button calls
`/api/dev_login` using `VITE_DEV_LOGIN_SECRET`.

Stopping `cargo dev up` with Ctrl+C also stops the local PostgreSQL container. Its
named volume is preserved, so the next run starts with the same database.

If you need to run only the frontend manually, use:

```sh
cd sakiot-stage
bun dev
```

The command above is only useful when running the frontend manually; normal
development starts both processes through `cargo dev up`.

Other subcommands:

```sh
cargo dev db up                                      # Postgres + migrate + seed
cargo dev up --fixtures prompt                       # choose skip/full/custom
cargo dev db down                                    # stop Postgres
cargo dev db reset                                   # drop the volume and re-seed
cargo dev fixtures sync                              # show counts, then choose latest recordings
cargo dev fixtures fetch <what>                     # copy one recording/session/clip/stamp
cargo dev clean --yes                                # drop the volume + delete managed fixtures
```

Shell completion and production-shaped fixture workflows live in
[Local fixtures and shell completion](docs/local-fixtures.md). That guide covers
bulk sync, targeted fetches, staging imports, identity rules, and remote overrides.

## Environment

`cargo dev` generates the local env files when absent; keep those generated
values for the managed local environment. For manual configuration only, copy
the example if `.env` does not already exist and fill in credentials:

```sh
test -e .env || cp .env.example .env
chmod 600 .env
```

The root `.env` is used by both Rust services, SQLx macros and CLI commands, and
the database backup scripts. For `cargo dev`, command-line flags override
exported variables, which override root `.env`, which override built-in
defaults. Set `SAKIOT_ENV_FILE` to override its path for backup jobs. Frontend
development values live in root `.env.development`; Vite loads environment
files from the monorepo root.

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
cargo dev db up
```

## Rust Workspace

The root Cargo workspace contains both services, three shared crates, and the
two operations CLIs. `sakiot-DSP` is a separate workspace compiled as a path
dependency of `web-server`; run its standalone tests separately:

```sh
cargo build --workspace
DATABASE_URL="$SAKIOT_TEST_DATABASE_URL" cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test --manifest-path sakiot-DSP/Cargo.toml --locked
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
cargo run -p web_server --bin web_server
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

The pre-commit hook formats Rust and frontend files, re-stages files already
staged, and checks SQLx metadata when relevant migrations or Rust files change.
The pre-push hook checks Rust formatting, workspace Clippy with warnings denied,
frontend Biome checks, and generated OpenAPI types. To fix formatting before
committing:

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

`cargo dev` sets `SAKIOT_DATA_DIR` to the repository's `data` directory.
Manual service runs resolve this variable relative to their working directory;
without it, the shared crate falls back to `../data`. Use an absolute path for
consistent behavior, for example `/data` in containers with a shared volume.

Component READMEs and the operations guides cover configuration and deployment.
Pushes to `main` auto-deploy to staging, except Markdown/license-only changes.
A workspace version bump triggers automatic tagging and production promotion
after staging succeeds. Production accepts strict `vX.Y.Z` tags; `ops/release`
is the manual fallback. See [deployment operations](ops/README.md),
[staging](STAGING.md), and [branch previews](PREVIEW.md).

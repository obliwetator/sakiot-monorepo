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

## Rust Workspace

The repository root is a Cargo workspace containing both services and both
shared crates.

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

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
```

## Runtime Media

Media defaults to the repository's `data` directory. Override this with
`SAKIOT_DATA_DIR`, for example `/data` in containers with a shared volume.

Each component has its own README with configuration and deployment details.

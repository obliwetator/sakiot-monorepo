# Sakiot

Sakiot is a Discord voice-recording system. A bot records voice channels, and a
web application lets members browse, play, clip and edit the recordings.

| Path | What it is |
| --- | --- |
| `fbi-agent` | Discord bot and voice recorder ([README](fbi-agent/README.md)) |
| `web-server` | HTTP API, auth, media serving and jobs ([README](web-server/README.md)) |
| `sakiot-stage` | React frontend ([README](sakiot-stage/README.md)) |
| `sakiot-dsp` | Clip-editor audio processing, native and WASM ([README](sakiot-dsp/README.md)) |
| `sakiot-db` | PostgreSQL migrations and backup scripts ([README](sakiot-db/README.md)) |
| `sakiot-paths` | Shared media file layout |
| `sakiot-permissions` | Permission rules: who may do what in a guild |
| `sakiot-proto` | Shared gRPC contract |
| `sakiot-storage` | Shared Backblaze B2 client |
| `ops/sakiot-dev` | `cargo dev`, the local environment |
| `ops/sakiot-deploy` | The deploy engine ([operations](ops/README.md)) |

Start with [architecture](docs/architecture.md): how the pieces fit, the
recording and playback paths, authorization, realtime updates, and the
contracts CI enforces.

## Local development

Requirements: Docker with Compose, Rust (the toolchain is pinned in
`rust-toolchain.toml`), `cargo-watch`, Bun, FFmpeg with `ffprobe`, `rsync` for
fixtures, and the pinned `wasm-bindgen` CLI for the DSP build:

```sh
cargo install wasm-bindgen-cli --version "$(cat sakiot-dsp/wasm-bindgen-cli-version)" --locked
```

Then one command starts everything:

```sh
cargo dev up
```

The first run generates a root `.env` (random JWT and dev-login secrets,
PostgreSQL on `localhost:54320`) and a `.env.development.local` pointing the
frontend at the local API. It then starts PostgreSQL from `compose.dev.yml`,
migrates, seeds a dev account and guild, and offers to pull fixtures (pass
`--fixtures skip` or `--fixtures full` when not interactive). It supervises
`web-server` under `cargo watch` and the frontend under Bun, and Ctrl+C stops
both along with PostgreSQL; the database volume is kept. Sign in with the dev
login button and the `DEV_LOGIN_SECRET` from `.env`; no Discord application is
needed.

```sh
cargo dev db up          # PostgreSQL + migrate + seed only
cargo dev db down        # stop PostgreSQL
cargo dev db reset       # drop the volume and re-seed
cargo dev fixtures sync  # pull recordings, clips and stamps from a deployed instance
cargo dev clean --yes    # drop the volume and managed fixtures
```

Fixtures, staging imports and shell completion are covered in
[local fixtures](docs/local-fixtures.md).

### Environment

Command-line flags override exported variables, which override the root
`.env`, which overrides built-in defaults. Both services, SQLx and the backup
scripts read the root `.env`; Vite reads `.env.development` from the root too.
For a manual setup, `cp .env.example .env && chmod 600 .env` and fill it in.
`.env.example` documents every variable.

Media lives under `SAKIOT_DATA_DIR`, which `cargo dev` sets to `data/`.
Without it, services fall back to `../data` relative to their working
directory, so use an absolute path for manual runs.

## Checks

Enable the repository's hooks once per clone:

```sh
git config core.hooksPath .githooks
```

The pre-commit hook formats staged Rust and frontend files (only the staged
hunks) and checks `.sqlx` when migrations or SQLx queries change. The
pre-push hook runs `cargo fmt --check`, workspace Clippy with warnings denied,
Biome, and the generated API types check. CI runs all of that plus the tests,
selecting suites by what changed.

Database tests create a disposable database per test from
`SAKIOT_TEST_DATABASE_URL`, and fail with `PoolTimedOut` when PostgreSQL is not
running (`cargo dev db up`):

```sh
set -a; . ./.env; set +a
DATABASE_URL="$SAKIOT_TEST_DATABASE_URL" cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
(cd sakiot-stage && bun run test && bun run build)
```

Two generated files are committed and checked:

- `.sqlx`, the SQLx query metadata that lets builds run without a database.
  After changing a `query!` macro or the schema, run `scripts/sqlx-prepare.sh`
  (it uses its own throwaway PostgreSQL container).
- `sakiot-stage/src/api/openapi.ts`, the frontend's API types. After changing
  the API, run `bun run generate:api-types` in `sakiot-stage`.

## Deploying

Pushes to `main` deploy to staging, except Markdown or license-only changes.
Bumping `[workspace.package] version` in `Cargo.toml` also tags and promotes
that commit to production once staging verifies it. Other branches get a
preview slot. [Operations](ops/README.md) covers environments, releases,
rollback and the VPS.

## License

MIT. See [LICENSE](LICENSE).

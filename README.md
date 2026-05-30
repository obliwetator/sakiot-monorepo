# FBI Agent

FBI Agent is the Discord bot part of the Sakiot system. It connects to Discord,
joins voice channels, records voice activity, handles bot commands, writes
recording metadata to Postgres, and exposes gRPC services used by the rest of
the stack.

This project is functional, but it is not packaged as a supported application.
No support is provided for running, deploying, configuring, or operating it. For
now, you have to figure that out yourself from the code, environment variables,
deployment scripts, local setup, and the shared database migrations in
`../sakiot-db`.

## Role In The System

FBI Agent is linked with the other projects in this directory to make the whole
Sakiot application:

- `web_server` reads the recordings and metadata produced by this bot and
  exposes them through HTTP APIs.
- `sakiot_stage` is the web UI that talks to `web_server`.
- `sakiot-paths` provides the shared recording path layout used by the bot and
  server.
- `sakiot-proto` provides the shared gRPC contract and generated Rust types used
  by the bot and server.
- `sakiot-db` owns the shared Postgres schema and migrations used by this service and `web_server`.

## What It Does

- Connects to Discord using Serenity and Songbird.
- Records Discord voice audio into the shared Sakiot recording layout.
- Stores guild, channel, user, recording, and runtime metadata in Postgres.
- Provides gRPC APIs for administration, dashboard state, snapshots, and bot
  control.
- Supports drain-aware release deployment through the scripts in `deploy/`.
- Emits telemetry and process metrics for observability.

Runtime media defaults to `../data` and can be moved by setting
`SAKIOT_DATA_DIR`, for example `SAKIOT_DATA_DIR=/data` in containers.
Existing local media should be moved into `../data/{voice_recordings,
no_silence_voice_recordings,waveform_data,clips}` during a planned downtime
window before deploying a build that uses the new default.

## Status

This is personal/project code, not a turnkey product. It may require specific
Discord application settings, shared database migrations, secrets, filesystem
paths, systemd units, and matching versions of the sibling projects.

## Database Migrations

Database migrations are owned by the shared `../sakiot-db` project. This
service does not keep service-local migrations and does not run migrations on
startup.

```sh
cd ../sakiot-db
sqlx migrate info --source migrations
sqlx migrate run --source migrations
```

# Web Server

Web Server is the Rust HTTP API for the Sakiot system. It serves authentication,
Discord user and guild data, recording metadata, audio files, live playback
state, waveform data, clips, stamps, admin controls, OpenAPI output, and runtime
connections to FBI Agent instances.

This project is functional, but it is not packaged as a supported application.
No support is provided for running, deploying, configuring, or operating it. For
now, you have to figure that out yourself from the code, environment variables,
the shared database migrations in `../sakiot-db`, and local setup.

## Role In The System

Web Server is linked with the other projects in this directory to make the whole
Sakiot application:

- `FBI-agent` records Discord voice audio and writes the metadata this server
  reads.
- `sakiot_stage` is the frontend that consumes this server's API.
- `sakiot-paths` provides the shared path layout used to find recordings,
  waveform data, live streams, and clips.
- `sakiot-proto` provides the shared gRPC contract and generated Rust types used
  to talk to FBI Agent instances.

## What It Does

- Exposes the `/api` HTTP routes used by the frontend.
- Handles Discord OAuth, JWT cookies, refresh, logout, and protected API access.
- Serves recorded audio, live HLS playback, waveform data, no-silence output,
  and downloadable recordings.
- Manages clips, stamps, cooldown admin settings, and dashboard streams.
- Registers and queries FBI Agent gRPC endpoints.
- Publishes OpenAPI documentation at `/api-doc/openapi.json` and Scalar at
  `/scalar`.
- Emits HTTP metrics and telemetry for observability.

Runtime media defaults to `../data` and can be moved by setting
`SAKIOT_DATA_DIR`, for example `SAKIOT_DATA_DIR=/data` in containers.
Existing local media should be moved into `../data/{voice_recordings,
no_silence_voice_recordings,waveform_data,clips}` during a planned downtime
window before deploying a build that uses the new default.

## Status

This is personal/project code, not a turnkey product. It assumes a matching
database schema, Discord configuration, filesystem layout, secrets, and sibling
project versions.

## Database Migrations

Database migrations are owned by the shared `../sakiot-db` project. This
service does not keep service-local migrations and does not run migrations on
startup.

```sh
cd ../sakiot-db
sqlx migrate info --source migrations
sqlx migrate run --source migrations
```

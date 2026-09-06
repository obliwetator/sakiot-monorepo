# Web Server

Web Server is the Rust HTTP API for the Sakiot system. It serves authentication,
Discord user and guild data, recording metadata, audio files, live playback
state, waveform data, clips, stamps, admin controls, OpenAPI output, and runtime
connections to FBI Agent instances.

This project is functional, but it is not packaged as a supported application.
No support is provided for running, deploying, configuring, or operating it. For
now, you have to figure that out yourself from the code, environment variables,
the shared database migrations in `../sakiot-db`, and local setup.

## Local Debug

For a quick local edit loop, `cargo dev up --fixtures skip` starts Postgres,
runs migrations, seeds a dev account, starts the frontend, and runs this server
under `cargo watch` with the `dev-login` cargo feature (Discord OAuth bypass via
`GET /api/dev_login` and `DEV_LOGIN_SECRET`). See the root `README.md`.

When running manually:

```sh
cargo run -p web_server --features dev-login
```

## Role In The System

Web Server is linked with the other projects in this directory to make the whole
Sakiot application:

- `fbi-agent` records Discord voice audio and writes the metadata this server
  reads.
- `sakiot-stage` is the frontend that consumes this server's API.
- `sakiot-paths` provides the shared path layout used to find recordings,
  waveform data, live streams, and clips.
- `sakiot-proto` provides the shared gRPC contract and generated Rust types used
  to talk to FBI Agent instances.
- `sakiot-db` owns the shared Postgres schema and migrations used by this service and `fbi-agent`.

## What It Does

- Exposes the `/api` HTTP routes used by the frontend.
- Handles Discord OAuth, JWT cookies, refresh, logout, and protected API access.
- Serves recorded audio, live HLS playback, waveform data, no-silence output,
  and downloadable recordings.
- Manages clips, stamps, and cooldown admin settings.
- Registers and queries FBI Agent gRPC endpoints.
- Publishes OpenAPI documentation at `/api-doc/openapi.json` and Scalar at
  `/scalar`.
- Emits HTTP metrics and telemetry for observability.

Export the same OpenAPI document without starting the service or connecting to
the database:

```sh
SQLX_OFFLINE=true cargo run --locked -p web_server --bin export_openapi
```

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

## Durable composition exports

Composition exports are stored in PostgreSQL before the API returns `202`.
`POST /api/audio/clips/{guild_id}/compose` accepts an optional `Idempotency-Key`
header (1–128 ASCII letters, digits, or hyphens). Repeating the same key and body
for the same user/guild returns the existing job; changing the body returns
`409`. Send a new key for an intentionally new export. The returned `id` is a
job ID, separate from the destination clip ID.

`GET /api/audio/clips/{guild_id}/compose/{job_id}` is owner-scoped and read-only.
It returns `queued`, `running`, `ready`, or `failed`, plus `stage`, `progress`,
`error`, and `result_clip_id` (populated on success). Unknown, expired, and other
users' jobs return `404`. Terminal results and request keys remain available for
30 days. The editor stores pending requests per user/guild, reconnects after a
refresh, and safely retries a lost submission response with the original key.
Closing the export dialog does not cancel the job.

The web server supervises its own executable in an isolated Linux process
group (`compose-worker` is an internal command). No extra service or queue
broker is needed. PostgreSQL serializes claims across overlapping web releases.
Each accepted job runs with these conservative limits:

- One running composition per database; at most 100 queued/running jobs total
  and three per user.
- A 60-second lease, renewed every 15 seconds. Attempts that lose their lease
  cannot publish. A crash is retried from the beginning, up to three attempts;
  ordinary transient failures wait 10 seconds times the attempt number.
- A 30-minute attempt deadline and 2 GiB process address-space limit. FFmpeg
  inherits the child limits. A watchdog also stops the group if its supervisor
  disappears.
- An 8 GiB per-file limit and a total attempt-workspace budget checked every
  two seconds. Sources are pinned with hard links on the media filesystem.
  Budget failures fail the export instead of publishing partial output.

Source authorization and metadata are checked before enqueueing; source
hydration, unknown-duration probing, and rendering happen inside the bounded
worker. Source access and immutable filenames are checked again on execution.
Overwrite publication compares the destination's previous filename, so an
intervening overwrite or deletion produces a conflict instead of replacing
newer work. Output is flushed and renamed to an immutable filename before one
transaction publishes the clip, its archive revision, and job completion.
An ambiguous commit never causes immediate output deletion.

Temporary attempts live under `clips/.composition-jobs/`; completed audio lives
under `clips/compositions/`. The reconciler removes inactive attempt directories
and unreferenced composition outputs after an hour. It also removes superseded
local overwrite files after that grace period. It never deletes a filename
still referenced by a clip, including a soft-deleted clip. Archive eviction and
waveform caches use immutable file revisions, so stale work cannot target a
replacement file. Waveforms remain regenerable caches and are generated on
request after composition publication.

Apply migration `20260906000000_composition_jobs.sql` before running this
release. It is additive and protects archive revisions even while an older web
release is still running. Queued jobs use renderer contract version 1; future
renderer changes must retain compatibility or explicitly migrate pending jobs.
A rollback to a release without the worker leaves accepted jobs queued until a
compatible release runs again; do not drop the queue tables during rollback.

Inspect the queue without modifying it:

```sql
SELECT state, stage, count(*), min(created_at) AS oldest
FROM composition_jobs GROUP BY state, stage ORDER BY state, stage;

SELECT id, state, attempts, lease_expires_at, error
FROM composition_jobs WHERE state <> 'ready' ORDER BY created_at;
```

Tests use SQLx disposable databases and temporary media roots. The integration
suite runs the actual worker binary and FFmpeg, including process termination
and recovery. No test should use the runtime database.

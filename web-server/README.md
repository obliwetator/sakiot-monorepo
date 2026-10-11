# web-server

The HTTP API, authentication, media rendering and serving, background jobs, and
the gRPC client to `fbi-agent`. How it fits with the other components is in
[architecture](../docs/architecture.md); `cargo dev up` from the repository root
runs it locally. To run it by itself:

```sh
cargo run -p web-server --bin web_server --features dev-login
```

The `dev-login` feature adds `/api/dev_login` and the load-test harness
(`src/loadtest/`). Local, staging and preview builds enable it; production
builds never do. Staging and previews share production's VPS, so run load
tests against them only off-peak, when nobody is using the bot (see
[production priority](../ops/README.md#production-priority)).

## OpenAPI

The document is generated at compile time and served at
`/api-doc/openapi.json`, with Scalar at `/scalar`. Export it without starting
the service or connecting to a database:

```sh
SQLX_OFFLINE=true cargo run --locked -p web-server --bin export_openapi
```

## Request timing

Every request's latency is split into named segments (`auth`, `perm`,
`session`, `tree`, `plan` and so on) that feed the
`http_server_segment_duration` histogram. With
`SAKIOT_SERVER_TIMING_HEADER=true`, which local, staging and preview set and
production must not, the segments also come back as a `Server-Timing` header,
shown in the browser's DevTools under Network > Timing. A segment sums all its
calls and notes the count, so a query repeated per row stands out. Segments
may nest, so they do not add up to `total`. Wrap new work in
`server_timing::measure("name", future)` to add a segment.

## Composition exports

`POST /api/audio/clips/{guild_id}/compose` stores the export as a job in
PostgreSQL before answering `202`. It accepts an optional `Idempotency-Key`
header (1–128 ASCII letters, digits or hyphens): repeating the key with the
same body for the same user and guild returns the existing job, and a
different body returns `409`. The returned `id` is a job id, not the clip id.

`GET /api/audio/clips/{guild_id}/compose/{job_id}` is owner-scoped and returns
`queued`, `running`, `ready` or `failed`, with `stage`, `progress`, `error` and
`result_clip_id`. Unknown, expired and other users' jobs return `404`. Results
and keys are kept for 30 days. The editor remembers pending requests per user
and guild, reconnects after a reload, and retries a lost submission with the
original key. Closing the export dialog does not cancel the job.

The server runs each job in a child of its own executable (the internal
`compose-worker` command) in an isolated process group, with no extra service
or broker. PostgreSQL serializes claims across overlapping releases. Limits:

- One running composition per database; at most 100 queued or running jobs,
  and three per user.
- A 60-second lease renewed every 15 seconds. An attempt that loses its lease
  cannot publish. A crash retries from the beginning, up to three attempts;
  transient failures wait 10 seconds times the attempt number.
- A 30-minute attempt deadline and a 2 GiB address-space limit, inherited by
  FFmpeg. A watchdog stops the group if its supervisor disappears.
- An 8 GiB per-file limit and a total workspace budget checked every two
  seconds. Sources are pinned with hard links. Exceeding a budget fails the
  export instead of publishing partial output.

Authorization and metadata are checked before enqueueing and again when the
job runs. Output is flushed and renamed to an immutable file name before one
transaction publishes the clip, its archive revision and the job result. An
overwrite compares the destination's previous file name, so an intervening
overwrite or deletion becomes a conflict instead of replacing newer work.

Attempts work under `clips/.composition-jobs/` and finished audio lives under
`clips/compositions/`. After an hour the reconciler removes inactive attempt
directories, unreferenced outputs and superseded overwrite files, but never a
file a clip still references, soft-deleted clips included.

Jobs record the DSP renderer version they need (currently 2; see
[sakiot-dsp](../sakiot-dsp/README.md)). Before claiming, the worker upgrades
queued and expired version 1 jobs; a running version 1 attempt finishes as is.
A rollback to a release without this worker leaves accepted jobs queued, so
never drop the queue tables during a rollback.

```sql
SELECT state, stage, count(*), min(created_at) AS oldest
FROM composition_jobs GROUP BY state, stage ORDER BY state, stage;

SELECT id, state, attempts, lease_expires_at, error
FROM composition_jobs WHERE state <> 'ready' ORDER BY created_at;
```

The integration tests run the real worker binary and FFmpeg, including process
termination and recovery, against disposable databases and media roots.

## Recording lifecycle

Guild managers set retention and excluded voice channels at
`/api/admin/guilds/{guild_id}/recording-policy` (the Voice Settings page).
Retention is 1–3650 days, or off, which is the default. Any member can opt out
of being recorded in a server at
`/api/users/current/guilds/{guild_id}/recording-opt-out`, the same table the
bot's `/recording` command writes. Opting out keeps earlier recordings.

Retention looks only at finalized sessions, by `ended_at`, and soft-deletes at
most 25 expired sessions per minute.

`DELETE /api/admin/guilds/{guild_id}/recordings/{recording_session_id}` needs a
live Manage Guild grant and returns `202` with a stable, manager-only
`status_url`. By default it soft-deletes at once: the session and its clips
disappear from view while their rows and media stay, so the deletion is
recoverable and does not free storage. Repeating it is idempotent. A
`recording_deletion_jobs` row keeps the actor or retention reason, the mode,
the stage and the outcome. Restoring a soft-deleted session has no UI yet.

Permanent purging needs both `SAKIOT_RECORDING_PERMANENT_DELETE_ENABLED=true`
and an explicit manager request with `?mode=permanent`; the flag alone never
upgrades earlier soft deletions. The fenced worker removes every B2 version and
delete marker under each source prefix, the local originals and derivatives,
then the database rows, and records who authorized it and when. It is
irreversible and not offered in the UI. A failed or `paused` permanent job
stays hidden until the same request requeues it after review. Database backups
and other copies follow their own retention.

Clip-editor overwrites are tracked in `clip_source_history`, but versions
overwritten before the oldest retained composition job could not be linked
when that ledger was backfilled. Before purging a guild with old overwrites,
check those versions by hand rather than trusting the job's verification.

```sql
SELECT id, guild_id, recording_session_id, reason, requested_by,
       permanent_requested_by, permanent_requested_at, mode, state, stage,
       attempts, error, created_at, finished_at
FROM recording_deletion_jobs ORDER BY created_at DESC LIMIT 50;
```

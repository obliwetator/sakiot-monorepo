# Backblaze B2 media archive runbook

Status: **rollout complete**, confirmed by the repository owner on 2026-09-07.
The completion date was not recorded here. Provisioning and rollout steps below
remain as reference for new environments and recovery; they are not pending
production work.

Historical planning baseline (capture date not recorded; remeasure at cutover):
294 recordings, 8.68 GB of audio, 11 clips, with database rows and files matched
at that snapshot. This is not a current inventory; rollout completion is recorded above.

Pricing checked on 2026-09-07: B2 lists the first 10 GB free, paid storage from
$6.95/TB/month, and free egress up to three times average monthly storage.
Account usage includes other buckets and retained versions. See
[storage pricing](https://www.backblaze.com/cloud-storage/pricing).
Class A/B/C API calls are free; Class D has a daily free allowance and usage
charges. Recheck [transaction pricing](https://www.backblaze.com/cloud-storage/transaction-pricing)
and egress overage rates before rollout.

## Provisioning

1. Create or use a Backblaze account in **EU Central**. Account region is
   permanent; EU Central stores data in Amsterdam:
   <https://www.backblaze.com/docs/cloud-storage-data-regions>.
2. Create globally unique private buckets:
   - `sakiot-media-prod-<random>`
   - `sakiot-media-staging-<random>`
   - `sakiot-db-backups-<random>`
3. Enable default SSE-B2 (AES-256) before uploading anything. Keep Object Lock
   disabled. Leave bucket lifecycle rules unset; application-controlled
   recording retention must explicitly remove all versions of deleted media.
   SSE applies only to uploads made after it is enabled:
   <https://www.backblaze.com/docs/cloud-storage-server-side-encryption>.
4. Create granular, bucket-restricted keys with the B2 CLI (or
   `b2_create_key`) from a trusted administrator workstation. The currently
   deployed media key allows deletion (operator-confirmed); verify each
   environment's key before enabling lifecycle deletion. Media keys need
   `listAllBucketNames,listBuckets,listFiles,readFiles,writeFiles,deleteFiles`.
   The native-B2 rclone backup key needs
   `listBuckets,listFiles,readFiles,writeFiles`; it does not need deletion:

   ```sh
   media_caps=listAllBucketNames,listBuckets,listFiles,readFiles,writeFiles,deleteFiles
   backup_caps=listBuckets,listFiles,readFiles,writeFiles

   b2 key create --bucket sakiot-media-prod-<random> \
     sakiot-media-prod "$media_caps"
   b2 key create --bucket sakiot-media-staging-<random> \
     sakiot-media-staging "$media_caps"
   b2 key create --bucket sakiot-db-backups-<random> \
     sakiot-db-backups "$backup_caps"
   b2 key list --long
   ```

   `listAllBucketNames` is required when S3 `List Buckets` compatibility is
   needed with a bucket-restricted key. `writeFiles` includes starting,
   finishing, and aborting multipart uploads. Save each returned secret at
   creation; it is shown once:
   <https://www.backblaze.com/docs/cloud-storage-application-key-capabilities>.

   Important B2 distinction: an S3 `DeleteObject` without a version ID adds a
   delete marker and leaves older versions recoverable. `deleteFiles` permits
   permanent deletion of a specific version. Recording deletion must enumerate
   and remove every version (including delete markers) of each owned object;
   hiding only the current name does not meet the privacy requirement. This
   permission makes mistakes irreversible, so the optional permanent mode is
   manager-only, narrowly keyed, audited, retryable, and disabled by default.
   That mode uses version-ID deletes; a plain S3 delete is not sufficient.
5. Put production/staging media credentials only in their root-owned
   `/etc/sakiot/*.env` files. Never deploy account master credentials.
6. Configure `/etc/sakiot/rclone.conf` with a native `b2` remote using the
   backup-bucket application key. Set `B2_BACKUP_REMOTE` to
   `remote-name:sakiot-db-backups-<random>` (not merely `remote-name:`).

Recommended permissions:

```sh
chown root:sakiot /etc/sakiot/production.env /etc/sakiot/staging.env /etc/sakiot/rclone.conf
chmod 0640 /etc/sakiot/production.env /etc/sakiot/staging.env /etc/sakiot/rclone.conf
```

## Provisioning smoke test

Use temporary environment variables or an AWS CLI profile containing only a
bucket-restricted media key:

```sh
aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api head-bucket \
  --bucket "$SAKIOT_MEDIA_S3_BUCKET"
printf test > /tmp/sakiot-b2-smoke
aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3 cp \
  /tmp/sakiot-b2-smoke "s3://$SAKIOT_MEDIA_S3_BUCKET/provisioning/smoke"
aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3 cp \
  "s3://$SAKIOT_MEDIA_S3_BUCKET/provisioning/smoke" /tmp/sakiot-b2-smoke.download
cmp /tmp/sakiot-b2-smoke /tmp/sakiot-b2-smoke.download
```

Use a fresh, dedicated smoke object to verify the media key can delete one
specific version. This command permanently deletes that test version; do not
run it against a recording or a reused object name:

```sh
delete_smoke_key="provisioning/delete-smoke-$(date +%s)-$$"
aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api put-object \
  --bucket "$SAKIOT_MEDIA_S3_BUCKET" --key "$delete_smoke_key" \
  --body /tmp/sakiot-b2-smoke
version_id="$(aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api head-object \
  --bucket "$SAKIOT_MEDIA_S3_BUCKET" --key "$delete_smoke_key" \
  --query VersionId --output text)"
test -n "$version_id" && test "$version_id" != None
aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api delete-object \
  --bucket "$SAKIOT_MEDIA_S3_BUCKET" --key "$delete_smoke_key" \
  --version-id "$version_id"
```

Confirm bucket remains private, default encryption reports AES-256, Object Lock
is disabled, and lifecycle retains every version in Backblaze console.

## Recording lifecycle deletion

Apply both the recording-lifecycle and recording-soft-delete migrations before
deploying matching web and FBI-agent binaries. Stop the old web deletion worker
before the soft-delete migration: it does not honor modes. Existing unfinished
permanent jobs are moved to `paused`, never resumed automatically, because an
earlier attempt may already have removed media. Review them individually.
Deploy both binaries before allowing managers to change recording policy; an
older bot will not enforce channel exclusions. Do not roll back to older web
binaries while soft-deleted sessions exist. Production retention stays off
until a guild manager explicitly sets `retention_days`.

Guild managers set retention (1–3650 days, or null/off) and excluded voice
channels on the Voice Settings page. New fragments in an excluded channel are
rejected; an already-running recorder checks policy each second and stops when
the channel becomes excluded. Retention considers only finalized sessions and
uses their `ended_at` timestamp. It soft-deletes at most 25 expired sessions per
minute. Soft deletion hides sessions and related clips, but does not itself
remove database records, local files, archive versions, or backups. Normal
archive cache pruning may still evict a local copy; archived bytes remain.
Soft deletion does not reduce durable storage usage. A restoration action is
not yet exposed.

`DELETE /api/admin/guilds/{guild_id}/recordings/{session_id}` requires a live
Manage Guild grant and returns `202` with a stable `status_url`. By default its
status is immediately `soft_deleted`; the session and related clips are hidden
and the media is untouched. The surviving `recording_deletion_jobs` row records
the original actor or retention reason, mode, stage, and outcome. Repeating the default
DELETE is idempotent and cannot trigger permanent purging.

Permanent deletion is reserved for a separately approved operation: set
`SAKIOT_RECORDING_PERMANENT_DELETE_ENABLED=true` on the web server **and** make
an explicit manager-authorized `DELETE .../recordings/{session_id}?mode=permanent`
request. The flag alone never upgrades soft-deleted jobs or resumes paused
legacy jobs. Permanent mode queues the existing fenced worker, which removes
every B2 version and delete marker under each source prefix, local originals
and derivatives, then database records. Separate audit fields record who
authorized permanent purging and when, even if retention hid the recording
first. Its audit row remains. This operation
is irreversible, and is not offered in the current UI. A failed or paused
permanent job remains hidden and needs explicit review before the same
`mode=permanent` request requeues it. The status URL is manager-only.

Check work and failures without changing state:

```sql
SELECT id, guild_id, recording_session_id, reason, requested_by,
       permanent_requested_by, permanent_requested_at, mode, state, stage,
       attempts, error, created_at, finished_at
FROM recording_deletion_jobs ORDER BY created_at DESC LIMIT 50;
```

Before invoking permanent mode on a guild with old clip-editor overwrites, audit
historical derivatives: the migration backfills provenance from current clip
composition and retained composition jobs, but versions overwritten before
the oldest retained job cannot be linked retrospectively. Investigate those
legacy versions manually; do not claim their purge was verified by the job.
The persistent `clip_source_history` ledger covers future overwrites even
after editor jobs age out. Database backups and external copies are separate
retention domains and must follow their own deletion policy.

## Staging rollout

1. Run migrations and deploy with `SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=0`.
2. Inspect without mutation:

   ```sh
   sudo -u sakiot /bin/bash -lc 'set -a; . /etc/sakiot/staging.env; set +a; /srv/sakiot-staging/current/web/web_server media migrate --dry-run'
   ```

3. Upload and full-hash verify every eligible object:

   ```sh
   sudo -u sakiot /bin/bash -lc 'set -a; . /etc/sakiot/staging.env; set +a; /srv/sakiot-staging/current/web/web_server media migrate --wait'
   sudo -u sakiot /bin/bash -lc 'set -a; . /etc/sakiot/staging.env; set +a; /srv/sakiot-staging/current/web/web_server media verify'
   sudo -u sakiot /bin/bash -lc 'set -a; . /etc/sakiot/staging.env; set +a; /srv/sakiot-staging/current/web/web_server media status'
   ```

4. Move selected verified local files aside (do not delete them yet), then test
   remote-only GET, HEAD, browser seeking/ranges, downloads, waveforms, silence
   removal, physical/logical clips, logical-session composition, `/jam`, and
   gRPC jam playback.
5. Temporarily block B2 endpoint. Recording must continue locally; remote-only
   media must return 503. `/livez` must remain healthy; `/healthz` now reflects
   both database and archive backlog, so it should become unready if due
   archive work waits more than one hour. Restore connectivity and confirm
   backlog and readiness recover.
6. Restore moved files or run `media restore --all`. Only after all tests pass,
   enable pruning in staging.

## Production rollout

1. Deploy with pruning disabled. Capture one DB/file count-and-byte snapshot.
2. Run `media migrate --wait`, `media verify`, and `media status`. Require
   eligible count = available count, missing = 0, conflict = 0, pending = 0,
   and snapshot byte/hash totals to match B2.
3. Verify current age private key has an encrypted, access-restricted off-host
   copy. Run one restore using that copy.
4. Run an encrypted backup. Confirm B2 copy exists, then run monthly-style
   `restore-test.sh`, which downloads newest B2 dump before restoring.
5. Declare this release rollback floor. Set
   `SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=1` and restart only through normal deploy
   procedure. Seven-day deletion deadlines begin at each object's full remote
   verification time.

## Rollback

Rollback to archive-aware code needs no media restore. For code older than this
feature:

1. Set `SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=0` and keep archive-aware binaries
   running.
2. Run `web_server media restore --all`.
3. Verify local counts, bytes, and SHA-256 values against `media_objects`.
4. Roll back application code. B2 objects and versions remain untouched.

Never use `rclone sync` for DB backups. `rclone copy` skips identical files and
does not delete destination files:
<https://rclone.org/commands/rclone_copy/>. Native B2 transfers verify SHA-1 on
upload/download: <https://rclone.org/b2/>.

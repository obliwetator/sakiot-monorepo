# Backblaze B2 media archive runbook

Planning baseline (remeasure at cutover): 294 recordings, 8.68 GB of audio,
11 clips, with database rows and files fully matched. This is below B2's
account-wide first 10 GB free allowance. Current paid storage is $6.95/TB/month
and free egress is up to three times average monthly storage:
<https://www.backblaze.com/cloud-storage/pricing>. Recheck API call allowances
and overage rates before rollout:
<https://www.backblaze.com/cloud-storage/transaction-pricing>.

## Provisioning

1. Create or use a Backblaze account in **EU Central**. Account region is
   permanent; EU Central stores data in Amsterdam:
   <https://www.backblaze.com/docs/cloud-storage-data-regions>.
2. Create globally unique private buckets:
   - `sakiot-media-prod-<random>`
   - `sakiot-media-staging-<random>`
   - `sakiot-db-backups-<random>`
3. Enable default SSE-B2 (AES-256) before uploading anything. Keep Object Lock
   disabled. Leave lifecycle rules unset so every object version is retained.
   SSE applies only to uploads made after it is enabled:
   <https://www.backblaze.com/docs/cloud-storage-server-side-encryption>.
4. Do not use the web console's `Read and Write` preset: it includes
   `deleteFiles`. Create three granular, bucket-restricted keys with the B2 CLI
   (or `b2_create_key`) from a trusted administrator workstation. The media
   keys need `listAllBucketNames,listBuckets,listFiles,readFiles,writeFiles`.
   The native-B2 rclone backup key needs
   `listBuckets,listFiles,readFiles,writeFiles`. Neither list includes
   `deleteFiles`:

   ```sh
   media_caps=listAllBucketNames,listBuckets,listFiles,readFiles,writeFiles
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

   Important B2 limitation: `writeFiles` also permits hiding a file, so an S3
   `DeleteObject` without a version ID may create a delete marker. Omitting
   `deleteFiles` prevents permanent deletion of stored versions, not hiding the
   current name. The application contains no delete-object operation, lifecycle
   deletion remains disabled, and retained versions permit administrator
   recovery.
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

Capture the uploaded object's version ID, then prove permanent version deletion
is denied. Success is a nonzero command with an access-denied response; stop if
version deletion succeeds:

```sh
version_id="$(aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api head-object \
  --bucket "$SAKIOT_MEDIA_S3_BUCKET" --key provisioning/smoke \
  --query VersionId --output text)"
test -n "$version_id" && test "$version_id" != None
if aws --endpoint-url "$SAKIOT_MEDIA_S3_ENDPOINT" s3api delete-object \
    --bucket "$SAKIOT_MEDIA_S3_BUCKET" --key provisioning/smoke \
    --version-id "$version_id"; then
  echo 'FATAL: runtime credential can permanently delete B2 versions' >&2
  exit 1
fi
```

Confirm bucket remains private, default encryption reports AES-256, Object Lock
is disabled, and lifecycle retains every version in Backblaze console.

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
   media must return 503; `/healthz` must continue reflecting database health.
   Restore connectivity and confirm backlog resumes.
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

# Backblaze B2 media archive

Recordings and clips age out of local disk into a private B2 bucket per
environment; [architecture](../docs/architecture.md#media-lifecycle) describes
the flow. This runbook covers provisioning, enabling the archive in an
environment, and moving media back to local disk.

## Provisioning

1. Use a Backblaze account in **EU Central** (Amsterdam). An account's region
   is permanent.
2. Create globally unique private buckets:
   - `sakiot-media-prod-<random>`
   - `sakiot-media-staging-<random>`
   - `sakiot-db-backups-<random>`
3. Enable default SSE-B2 (AES-256) before uploading anything; it applies only
   to later uploads. Keep Object Lock disabled and leave lifecycle rules unset:
   recording deletion removes versions itself.
4. From a trusted administrator workstation, create bucket-restricted keys.
   Media keys need delete; the backup key must not have it:

   ```sh
   media_caps=listAllBucketNames,listBuckets,listFiles,readFiles,writeFiles,deleteFiles
   backup_caps=listBuckets,listFiles,readFiles,writeFiles

   b2 key create --bucket sakiot-media-prod-<random> sakiot-media-prod "$media_caps"
   b2 key create --bucket sakiot-media-staging-<random> sakiot-media-staging "$media_caps"
   b2 key create --bucket sakiot-db-backups-<random> sakiot-db-backups "$backup_caps"
   b2 key list --long
   ```

   `listAllBucketNames` lets a bucket-restricted key use S3 `ListBuckets`, and
   `writeFiles` covers multipart uploads. Each secret is shown once.
5. Put the media credentials (`SAKIOT_MEDIA_S3_*`) only in the root-owned
   `/etc/sakiot/*.env` files. Never deploy account master credentials.
6. Configure `/etc/sakiot/rclone.conf` with a native `b2` remote using the
   backup key, and set `B2_BACKUP_REMOTE` to
   `remote-name:sakiot-db-backups-<random>` (not just `remote-name:`).

```sh
chown root:sakiot /etc/sakiot/production.env /etc/sakiot/staging.env /etc/sakiot/rclone.conf
chmod 0640 /etc/sakiot/production.env /etc/sakiot/staging.env /etc/sakiot/rclone.conf
```

### Deletion and versions

In B2, an S3 `DeleteObject` without a version id only adds a delete marker;
older versions stay recoverable. Permanent recording deletion therefore lists
and deletes every version and delete marker of each object it owns, which needs
`deleteFiles` and cannot be undone. That mode is manager-only, audited and off
by default (see [recording lifecycle](../web-server/README.md#recording-lifecycle)).

### Smoke test

With an AWS CLI profile or environment holding only a bucket-restricted media
key:

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

Check that the key can delete one specific version, using a fresh object.
This permanently deletes that test version; never point it at a recording:

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

In the Backblaze console, confirm the bucket is private, default encryption
reports AES-256, Object Lock is off, and lifecycle keeps every version.

## Enabling the archive in an environment

`SAKIOT_MEDIA_ARCHIVE_ENABLED=1` requires every `SAKIOT_MEDIA_S3_*` value, and
startup fails on a partial configuration. Local copies are pruned only with
`SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=1`, so enable that last. Run the `media`
commands with the instance's environment loaded, for example on staging:

```sh
media() {
  sudo -u sakiot /bin/bash -lc "set -a; . /etc/sakiot/staging.env; set +a; \
    /srv/sakiot-staging/current/web/web_server media $*"
}
```

1. Deploy with pruning disabled. On production, capture a count-and-byte
   snapshot of the database rows and files.
2. `media migrate --dry-run` to inspect, then `media migrate --wait`,
   `media verify` and `media status`. Require eligible = available,
   missing = 0, conflict = 0 and pending = 0, and on production the snapshot's
   bytes and hashes to match B2.
3. Move some verified local files aside (do not delete them) and test
   remote-only playback: GET, HEAD, seeking and ranges, downloads, waveforms,
   silence removal, clips, session composition and `/jam`.
4. Block the B2 endpoint for a while. Recording must continue locally,
   remote-only media must answer 503, and `/livez` must stay healthy;
   `/readyz` turns unready once due archive work is over an hour old. Restore
   connectivity and confirm the backlog drains.
5. Put the moved files back (or `media restore --all`). On production, confirm
   the age key has an off-host copy and that `restore-test.sh` passes against
   the newest B2 dump.
6. Set `SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=1` and redeploy. Each object's
   seven-day local deadline starts at its full remote verification.

## Moving media back to local disk

To stop relying on the archive, set `SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED=0`, keep
the archive-aware release running, and run `media restore --all`. Verify local
counts, bytes and SHA-256 values against `media_objects` before changing
anything else. B2 objects and versions are left untouched. Single items restore
with `media restore --audio-file-ids <ids>` or `--clip-ids <ids>`.

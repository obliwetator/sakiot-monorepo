# Production data migration

Move the production recording archive from its temporary legacy location into
the canonical production directory.

## Current layout

```text
Service path:  /var/lib/sakiot/data
Bind source:   /home/tulipan/projects/sakiot/data
Database:      local PostgreSQL; unchanged by this migration
Archive size:  about 61 GiB
Filesystem:    both paths are on /dev/vda3
Free space:    about 137 GiB as of 2026-06-08
```

`/var/lib/sakiot/data` is currently a bind mount. Files physically remain under
the legacy project path. Because both locations use the same filesystem, the
archive can be renamed into place without copying 61 GiB.

## Preconditions

- No deployment or rollback is running.
- No recording is active.
- Current bot and web services are healthy.
- `/var/lib/sakiot/data` still resolves to the legacy bind source.
- The legacy bind entry still exists in `/etc/fstab`.
- No process other than Sakiot uses the legacy data path.

Do not start migration based only on an empty website. Check the bot directly:

```sh
grpcurl -plaintext \
  -import-path sakiot-proto/proto \
  -proto fbi_agent.proto \
  127.0.0.1:<current-grpc-port> \
  fbi_agent.Admin/GetDrainStatus
```

Required result before filesystem changes:

```text
activeRecordings: 0
```

## Migration

1. Acquire the deployment lock so a release cannot overlap the migration.
2. Ask the bot to enter drain mode. This prevents new voice sessions.
3. Recheck until both active recordings and active voice connections are zero.
4. Stop the production bot and web services.
5. Back up `/etc/fstab`.
6. Record the current bot unit, gRPC address, bind source, ownership, ACLs, and
   file counts.
7. Remove only the Sakiot legacy bind entry from `/etc/fstab`.
8. Unmount `/var/lib/sakiot/data`.
9. Rename the now-visible underlying production directory to a timestamped
   backup:

   ```text
   /var/lib/sakiot/data.pre-migration-<timestamp>
   ```

10. Rename the legacy archive into the canonical location:

    ```text
    /home/tulipan/projects/sakiot/data
      -> /var/lib/sakiot/data
    ```

11. Normalize the migrated tree ownership to `sakiot:sakiot`. Preserve
    directory traversal and service read/write permissions.
12. Validate:

    - `/var/lib/sakiot/data` is a normal directory, not a mount point.
    - The old `/home/tulipan/projects/sakiot/data` path no longer exists.
    - Recording, clip, waveform, and no-silence directories exist.
    - File counts match the pre-migration counts.
    - `/etc/fstab` parses successfully.

13. Restart the current bot and web services.
14. Confirm web health, database readiness, bot active role, and zero unexpected
    recording errors.
15. Cancel drain if the bot process was not restarted. A normal service restart
    starts it in active mode.

Expected service downtime: seconds to a few minutes. The directory rename itself
is atomic and does not copy file contents. Recursive ownership normalization is
the longest stopped-service step.

## Verification

```sh
findmnt /var/lib/sakiot/data
mountpoint /var/lib/sakiot/data
systemctl is-active sakiot-web.service
systemctl list-units 'sakiot-fbi-agent@*' --state=running
curl -fsS http://127.0.0.1:8900/healthz
```

Expected:

- `findmnt` shows no dedicated bind mount for `/var/lib/sakiot/data`.
- `mountpoint` reports that `/var/lib/sakiot/data` is not a mount point.
- Web and current bot units are active.
- Health response reports `status: ok` and `database: ready`.
- Website recordings and clips open successfully.
- A new test recording is written under `/var/lib/sakiot/data`.

Keep `data.pre-migration-<timestamp>` until website playback, clip playback,
new recording creation, and the next host reboot have all been verified.

## Rollback

If validation or service startup fails:

1. Stop bot and web services.
2. Move `/var/lib/sakiot/data` back to
   `/home/tulipan/projects/sakiot/data`.
3. Move `data.pre-migration-<timestamp>` back to `/var/lib/sakiot/data`.
4. Restore the `/etc/fstab` backup.
5. Recreate the bind mount at `/var/lib/sakiot/data`.
6. Restart bot and web services.
7. Verify web health and bot status.

Do not delete either tree during rollback. Do not change `DATABASE_URL`; the
database is already local and this migration changes filesystem placement only.

## Execution

Implement and run the migration as a guarded root script after
`activeRecordings` reaches zero. The script must validate every precondition,
hold the deployment lock, and automatically execute the rollback sequence on
failure or interruption.

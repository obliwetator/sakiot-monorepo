# Database backups

Encrypted logical backups of the production database, `sakiot_rouvas`.
[Operations](../../../ops/README.md#database-backups) covers installing the
scripts and enabling their systemd timers (hourly, nightly, and a monthly
restore test).

- **Logical dumps only.** `pg_dump -Fc` on a schedule, encrypted with `age`,
  kept on local disk and copied to B2. There is no WAL archiving; see
  [enabling PITR](#enabling-pitr) for when to add it.
- **B2 is mandatory.** A run succeeds only once every completed `*.dump.age`
  is in the private B2 bucket.

## Setup on a fresh host

Keep the existing private key on an existing host. On a new one:

```sh
age-keygen -o /etc/sakiot/age-key.txt   # prints the public key
chown root:sakiot /etc/sakiot/age-key.txt
chmod 0640 /etc/sakiot/age-key.txt
```

Put the public key in `AGE_RECIPIENT`, copy the key file off the host, and do
one restore with that copy before relying on it. Configure the native B2 remote
with the bucket-restricted, non-delete key (see the
[media archive runbook](../../../ops/B2_MEDIA_ARCHIVE.md#provisioning)):

```sh
sudo rclone config --config /etc/sakiot/rclone.conf
sudo chown root:sakiot /etc/sakiot/rclone.conf
sudo chmod 0640 /etc/sakiot/rclone.conf
```

Smoke test from the monorepo root as `sakiot`:

```sh
SAKIOT_ENV_FILE=/etc/sakiot/production.env sakiot-db/ops/backup/backup.sh hourly
SAKIOT_ENV_FILE=/etc/sakiot/production.env sakiot-db/ops/backup/restore-test.sh
```

## Scripts

Each script loads `SAKIOT_ENV_FILE` (default: the monorepo root `.env`) and
fails fast when a required variable is unset.

### `backup.sh [hourly|nightly|pre-migrate]`

1. Takes a `flock`, so runs never overlap.
2. Streams `pg_dump -Fc "$BACKUP_DATABASE_URL" | age -r "$AGE_RECIPIENT"` to
   `BACKUP_DIR/sakiot_rouvas_<label>_<YYYY-MM-DD_HHMM>.dump.age`. **No plaintext
   dump ever touches disk.**
3. Writes to a `.partial` file and renames it only on success; with
   `pipefail`, a dump that dies mid-stream is deleted, never promoted.
4. `rclone copy`s every completed dump. An upload or checksum failure fails the
   run; the remote is never synchronized or deleted from.
5. Prunes local dumps past their label's retention (hourly 7 days, nightly and
   pre-migrate 90).
6. Pings `HEALTHCHECK_URL` on success, if set (a dead-man switch).

Dumps are `chmod 600` and `BACKUP_DIR` is `chmod 700`.

### `restore.sh <file.dump.age> <target_db> [--force]`

Decrypts and runs `pg_restore --clean --if-exists --no-owner` into
`target_db`. It refuses to overwrite `sakiot_rouvas` without `--force`. For a
single table:

```sh
age -d -i /etc/sakiot/age-key.txt FILE.dump.age | pg_restore -t TABLE -d sakiot_rouvas
```

### `restore-test.sh`

Downloads the newest nightly from B2 (another label if there is no nightly),
restores it into the throwaway `sakiot_rouvas_restoretest`, checks the tables
came back, warns about pending migrations, prints row counts and drops the
database. **An untested backup is not a backup.**

### `pre-migrate-backup.sh`

Takes a `pre-migrate` dump, then runs `sqlx migrate run`, so every schema
change has a rollback point. The deploy engine takes this backup before
production migrations.

## Security

- `AGE_KEY_FILE` is the keystone: losing it makes every backup unrecoverable,
  and leaking it exposes all data. Keep an access-restricted copy off the host
  and prove it with a restore.
- The B2 key is restricted to the backup bucket and deliberately cannot
  delete. Never use the account master key.
- Always `rclone copy`, never `rclone sync`: local retention must not delete
  remote history.
- `.env`, `*.dump` and `*.dump.age` are git-ignored; never commit them.

## Enabling PITR

Add WAL archiving when the database passes about 1 GB, or when hourly dumps get
slow or lock-heavy enough that a one-hour recovery point is no longer
acceptable. Until then it only produces gigabytes of near-empty WAL segments
and adds a way to halt the database.

1. In `postgresql.conf`: `wal_level = replica`, `archive_mode = on`,
   `archive_command = 'test ! -f /wal/%f && cp %p /wal/%f'`, and an
   `archive_timeout` matching the desired recovery point; restart.
2. Take periodic base backups:
   `pg_basebackup -D /backups/base -Ft -z -X stream`.
3. Monitor `archive_command`: repeated failures fill `pg_wal/` and **halt the
   database**. Prune archived WAL on a retention window.
4. Restore a base backup and replay WAL to a `recovery_target_time`.

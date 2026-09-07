# Sakiot DB backups

Automated, encrypted logical backups of the `sakiot_rouvas` Postgres database.

## What this is (and isn't)

- **Layer 1 — logical dumps only.** `pg_dump -Fc` on a schedule, encrypted with
  `age`, kept on local disk and copied to B2. Measure current database and dump
  sizes when planning retention; earlier size estimates are not live metrics.
- **No WAL archiving / PITR.** For a near-idle, tiny db it produces gigabytes of
  mostly-empty 16 MB WAL segments per day and adds a halt-risk failure mode for
  no real benefit. See **[Future: PITR](#future-enabling-pitr)** for when to add it.
- **B2 is mandatory.** A backup run succeeds only after every completed
  `*.dump.age` has been copied to private Backblaze B2 storage.

## Setup

Production installation and timers are covered in [operations](../../../ops/README.md).
The commands below run from the monorepo root.

```sh
# 1. tools: pg_dump, age, flock, sqlx, rclone
apt install age rclone

# 2. On a fresh host only; retain the existing private key on an existing host.
#    Create the service account/directories with ops/install-production.sh first.
#    This stores the private key in the file and prints the public key.
age-keygen -o /etc/sakiot/age-key.txt
chown root:sakiot /etc/sakiot/age-key.txt
chmod 0640 /etc/sakiot/age-key.txt
#   -> copy the "Public key: age1..." into AGE_RECIPIENT
#   -> back up /etc/sakiot/age-key.txt OFF this host
#   -> perform one restore using that off-host copy before cutover

# 3. config (from the monorepo root)
test -e .env || cp .env.example .env
$EDITOR .env      # fill DB, age, B2_BACKUP_REMOTE, and backup settings

# Configure native B2 remote with bucket-restricted, non-delete credentials.
# Keep file root-owned; grant read only to backup service group.
sudo rclone config --config /etc/sakiot/rclone.conf
sudo chown root:sakiot /etc/sakiot/rclone.conf
sudo chmod 0640 /etc/sakiot/rclone.conf

# 4. smoke test
sakiot-db/ops/backup/backup.sh hourly # writes one encrypted dump
sakiot-db/ops/backup/restore-test.sh # restores it into a scratch db and checks it
```

## Behavior

### `backup.sh [hourly|nightly|pre-migrate]`

1. Sources `SAKIOT_ENV_FILE` (default: monorepo root `.env`); fails fast if
   required vars are unset. Installed production units set it to
   `/etc/sakiot/production.env`.
2. Takes a `flock` so two runs never overlap.
3. Streams `pg_dump -Fc "$BACKUP_DATABASE_URL" | age -r "$AGE_RECIPIENT"` to
   `BACKUP_DIR/sakiot_rouvas_<label>_<YYYY-MM-DD_HHMM>.dump.age`.
   **No plaintext dump is ever written to disk.**
4. Writes to a `.partial` file and only `mv`s it into place on success. If
   `pg_dump` dies mid-stream, `pipefail` aborts the run and the trap deletes the
   partial — a truncated file is never promoted to a real backup.
5. Runs `rclone copy` for all completed encrypted dumps. Upload or checksum
   failure fails the unit; remote destination is never deleted or synchronized.
6. Prunes local dumps older than the per-label retention (default: hourly 7d,
   nightly 90d, pre-migrate 90d).
7. If `HEALTHCHECK_URL` is set, pings it on success (dead-man switch).

Files are `chmod 600`; `BACKUP_DIR` is `chmod 700`.

### `restore.sh <file.dump.age> <target_db> [--force]`
Decrypts and `pg_restore --clean --if-exists --no-owner` into `target_db`.
Refuses to overwrite the live `sakiot_rouvas` without `--force`.

Single table:
```sh
age -d -i /etc/sakiot/age-key.txt FILE.dump.age | pg_restore -t TABLE -d sakiot_rouvas
```

### `restore-test.sh`
Downloads the newest B2 nightly (falling back to another label when no nightly
exists), restores it into throwaway db
`sakiot_rouvas_restoretest`,
asserts tables came back, warns on pending migrations (`sqlx migrate info`),
prints row counts, drops the scratch db. **Untested backup = no backup.**

### `pre-migrate-backup.sh`
Takes a `pre-migrate` dump, then runs `sqlx migrate run`. Use this in prod in
place of bare `sqlx migrate run` so every schema change has a rollback point.

## Cron

Production uses the systemd timers installed by `ops/install-production.sh`;
they run as `sakiot` and write to `/var/lib/sakiot/backups`. Use cron only for a
standalone development installation. Edit `crontab -e` for the user that owns
`BACKUP_DIR` and can reach Postgres. Use absolute paths and point them at the
current checkout.

```cron
# m  h            dom mon dow  command
17   0-2,4-23     *   *   *    /path/to/sakiot-monorepo/sakiot-db/ops/backup/backup.sh hourly
17   3            *   *   *    /path/to/sakiot-monorepo/sakiot-db/ops/backup/backup.sh nightly
30   4            1   *   *    /path/to/sakiot-monorepo/sakiot-db/ops/backup/restore-test.sh
```

- **Hourly** at :17 every hour except 03:00 (the nightly covers that slot).
- **Nightly** at 03:17, kept 90 days for longer history.
- **Restore test** monthly on the 1st at 04:30.

If `age`/`sqlx` live outside the system `PATH`, prepend it in the crontab:
`PATH=/usr/bin:/home/tulipan/.cargo/bin` at the top of the file. Without
`HEALTHCHECK_URL`, set a `MAILTO=you@example.com` so cron mails failures.

## Future: enabling PITR

Add WAL archiving when **either**: the db passes ~1 GB, **or** hourly dumps get
slow/lock-heavy enough that 1-hour RPO is unacceptable. Then:

1. `postgresql.conf`: `wal_level = replica`, `archive_mode = on`,
   `archive_command = 'test ! -f /wal/%f && cp %p /wal/%f'`, an `archive_timeout`
   matched to desired RPO, restart.
2. Periodic base backup: `pg_basebackup -D /backups/base -Ft -z -X stream`.
3. Monitor `archive_command` — repeated failures fill `pg_wal/` and **halt the
   db**. Prune archived WAL on a retention window.
4. Restore = restore base backup + replay WAL to a `recovery_target_time`.

## Security

- `.env`, `*.dump`, `*.dump.age` are gitignored - never commit them.
- `AGE_KEY_FILE` (private key) is the keystone: a **lost** key makes every backup
  unrecoverable; a **leaked** key exposes all data. Store a copy off this host,
  access-restricted, and prove it by restoring with that copy.
- B2 application key is bucket-restricted to read/write/list capabilities and
  deliberately lacks delete capability. Never use account master key here.
- Always use `rclone copy`, never `sync`; local retention must not delete remote
  backup history.
- Standalone runs use the root `.env` (gitignored, `chmod 600`). Installed
  production jobs use `/etc/sakiot/production.env` with owner `root:sakiot` and
  mode `0640`; `SAKIOT_ENV_FILE` selects the configuration file.

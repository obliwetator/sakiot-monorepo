# Sakiot DB backups

Automated, encrypted logical backups of the `sakiot_rouvas` Postgres database.

## What this is (and isn't)

- **Layer 1 — logical dumps only.** `pg_dump -Fc` on a schedule, encrypted with
  `age`, kept on local disk. The db is ~12 MB; a compressed dump is ~200 KB.
- **No WAL archiving / PITR.** For a near-idle, tiny db it produces gigabytes of
  mostly-empty 16 MB WAL segments per day and adds a halt-risk failure mode for
  no real benefit. See **[Future: PITR](#future-enabling-pitr)** for when to add it.
- **Local storage only.** A disk failure loses the db *and* its backups. See
  **[Future: offsite](#future-offsite)**.

## Setup

```sh
# 1. tools (already installed here): pg_dump, age, flock, sqlx
apt install age            # if not present

# 2. encryption keypair (stores private key in the file, prints public key)
age-keygen -o /etc/sakiot/age-key.txt
#   -> copy the "Public key: age1..." into AGE_RECIPIENT
#   -> back up /etc/sakiot/age-key.txt OFF this host

# 3. config
cp ops/backup/backup.env.example ops/backup/backup.env
$EDITOR ops/backup/backup.env      # fill DATABASE_URL, PGPASSWORD, AGE_*, BACKUP_DIR

# 4. smoke test
ops/backup/backup.sh hourly        # writes one encrypted dump
ops/backup/restore-test.sh         # restores it into a scratch db and checks it
```

## Behavior

### `backup.sh [hourly|nightly|pre-migrate]`
1. Sources `backup.env`; fails fast if required vars are unset.
2. Takes a `flock` so two runs never overlap.
3. Streams `pg_dump -Fc "$DATABASE_URL" | age -r "$AGE_RECIPIENT"` to
   `BACKUP_DIR/sakiot_rouvas_<label>_<YYYY-MM-DD_HHMM>.dump.age`.
   **No plaintext dump is ever written to disk.**
4. Writes to a `.partial` file and only `mv`s it into place on success. If
   `pg_dump` dies mid-stream, `pipefail` aborts the run and the trap deletes the
   partial — a truncated file is never promoted to a real backup.
5. Prunes dumps older than the per-label retention (default: hourly 7d,
   nightly 90d, pre-migrate 90d).
6. If `HEALTHCHECK_URL` is set, pings it on success (dead-man switch).

Files are `chmod 600`; `BACKUP_DIR` is `chmod 700`.

### `restore.sh <file.dump.age> <target_db> [--force]`
Decrypts and `pg_restore --clean --if-exists --no-owner` into `target_db`.
Refuses to overwrite the live `sakiot_rouvas` without `--force`.

Single table:
```sh
age -d -i /etc/sakiot/age-key.txt FILE.dump.age | pg_restore -t TABLE -d sakiot_rouvas
```

### `restore-test.sh`
Restores the newest nightly into throwaway db `sakiot_rouvas_restoretest`,
asserts tables came back, warns on pending migrations (`sqlx migrate info`),
prints row counts, drops the scratch db. **Untested backup = no backup.**

### `pre-migrate-backup.sh`
Takes a `pre-migrate` dump, then runs `sqlx migrate run`. Use this in prod in
place of bare `sqlx migrate run` so every schema change has a rollback point.

## Cron

Edit `crontab -e` for the user that owns `BACKUP_DIR` and can reach Postgres.
Use absolute paths (cron has a minimal `PATH`); point them at this checkout.

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

## Future: offsite

Local-only is the weak point. To ship offsite, append one line to `backup.sh`
after the prune step (and add the var to `backup.env`):

```sh
# rclone copy "$BACKUP_DIR" "$RCLONE_REMOTE" --min-age 1m
```

Dumps are already `age`-encrypted, so the remote need not be trusted.

## Security

- `backup.env`, `*.dump`, `*.dump.age` are gitignored — never commit them.
- `AGE_KEY_FILE` (private key) is the keystone: a **lost** key makes every backup
  unrecoverable; a **leaked** key exposes all data. Store a copy off this host,
  access-restricted.
- DB password lives in `backup.env` / `.env` (both gitignored). Lock down file
  perms (`chmod 600`).

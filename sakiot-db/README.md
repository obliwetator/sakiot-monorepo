# sakiot-db

The single owner of the PostgreSQL schema that `fbi-agent` and `web-server`
share. Neither service runs migrations at startup: `cargo dev` applies them
locally and the deploy engine applies them on each deploy (after an encrypted
backup on production; see [backups](ops/backup/README.md)).

```sh
cd sakiot-db
sqlx migrate info --source migrations
sqlx migrate run --source migrations
cd ..
scripts/sqlx-prepare.sh   # refresh .sqlx against a disposable database
```

- Add schema changes only under `migrations/`, and commit the refreshed `.sqlx`
  with them; the pre-commit hook and CI check it.
- Keep one-off data or filesystem scripts out of migrations.
- Never edit a migration that has been applied anywhere; preview slots and
  production fail on its checksum.
- The baseline migration is a schema-only dump of the original live schema,
  without data or SQLx metadata.

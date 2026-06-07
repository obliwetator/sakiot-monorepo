# Sakiot DB

Canonical Postgres migrations for the Sakiot services.

## Role

`FBI-agent` and `web_server` share one Postgres schema. Migrations live here so
schema changes have one owner instead of being split by service.

## Commands

Run from this directory, or pass `--source /home/tulipan/projects/sakiot/sakiot-db/migrations`.

```sh
sqlx migrate info --source migrations
sqlx migrate run --source migrations
```

The baseline migration is a schema-only dump of the existing live schema. It
does not include application data and does not include SQLx migration metadata.

## Policy

- Add future schema changes only under `migrations/`.
- Do not add service-local migrations under `FBI-agent` or `web_server`.
- Data/filesystem one-off scripts should stay separate from schema migrations.
- Services connect to the migrated schema; they do not run migrations on startup.

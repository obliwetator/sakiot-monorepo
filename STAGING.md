# Staging

Staging is live on the **same VPS** as production, as a fully separate instance
under the shared `sakiot` user. Every push to `main` deploys it; production still
ships only on strict `vX.Y.Z` tags.

## Pipeline

```
push main ──▶ .github/workflows/deploy-staging.yml ──▶ ssh "staging <sha>"
          ──▶ ops/deploy stage <sha> ──▶ build (offline) ▸ migrate ▸ bot+web ▸ frontend
```

Verify a commit on staging, then cut production:

```sh
ops/release vX.Y.Z      # validates clean tree/branch/semver/no-dup + staging matches HEAD, then tags+pushes
```

## Layout (staging vs prod)

| thing            | production            | staging                          |
|------------------|-----------------------|----------------------------------|
| web port         | 8900                  | **8901**                         |
| database         | `sakiot_rouvas`       | **`sakiot_staging`** (separate)  |
| Discord bot      | RELEASE bot           | **DEBUG bot**                    |
| web unit         | `sakiot-web.service`  | `sakiot-staging-web.service`     |
| bot unit         | `sakiot-fbi-agent@<id>` | `sakiot-staging-fbi-agent@<id>` |
| data dir         | `/var/lib/sakiot`     | `/var/lib/sakiot-staging`        |
| releases         | `/srv/sakiot`         | `/srv/sakiot-staging`            |
| cache            | `/var/cache/sakiot`   | `/var/cache/sakiot-staging`      |
| env file         | `/etc/sakiot/production.env` | `/etc/sakiot/staging.env`  |
| frontend domain  | patrykstyla.com       | **debug.patrykstyla.com**        |
| frontend docroot | `/var/www/patrykstyla.com` | `/var/www/debug.patrykstyla.com` |

The deploy engine is target-agnostic: same engine (`ops/deploy-release.sh`, or
the `ops/sakiot-deploy` Rust binary when `SAKIOT_DEPLOY_ENGINE=rust`), driven by
the env file plus `SAKIOT_WEB_UNIT` / `SAKIOT_BOT_UNIT_PREFIX`.

## Setup facts / gotchas (learned during cutover)

- **Builds run offline.** `cargo` runs with `SQLX_OFFLINE=true` and the committed
  `.sqlx` metadata, so a build needs **no live DB**. Without it, sqlx's
  compile-time macros hit `DATABASE_URL` (the empty staging DB) and fail with
  `relation "..." does not exist`. After a `query!` change run
  `cargo sqlx prepare` and commit `.sqlx`, or the offline build errors with
  `no cached data for this query`.
- **DB backup skipped on staging.** `SAKIOT_SKIP_DB_BACKUP=1` in `staging.env`
  applies migrations without the encrypted pre-migrate backup (staging DB is
  disposable; no `age` key needed). The migration phase still seeds the DB on
  first deploy. Reset anytime: `dropdb sakiot_staging && createdb -O sakiot sakiot_staging`.
- **Discord bot token is selected at compile time** (`FBI-agent/src/config.rs`,
  `#[cfg(debug_assertions)]`). A `--release` build reads the `*_RELEASE*` slots,
  so `staging.env` puts the **DEBUG** bot's token/app-id in
  `DISCORD_TOKEN_RELEASE` / `APPLICATION_ID_RELEASE`. Don't run a manual debug
  bot with that token while staging is up (one gateway connection per token).
- **Cutover:** the old dev-login debug web (`web_server-debug.service`, tulipan
  `--user`, on 8901) was disabled so staging owns 8901 + debug.patrykstyla.com.
- **Frontend API origin** is baked at build time from `VITE_API_URL` in
  `staging.env` (`https://debug.patrykstyla.com/api/`). Vite reads `VITE_*` from
  the (exported) env. NOTE: `Constants.ts` and `features/metrics/hooks.ts` still
  **hardcode `dev.patrykstyla.com`** — the metrics dashboard websocket streams
  from prod on staging until those move to `VITE_API_URL`.

## nginx (debug.patrykstyla.com vhost)

Must serve the staging docroot and proxy the API to staging web:

```nginx
root /var/www/debug.patrykstyla.com;          # NOT /var/www/patrykstyla.com
location /api/ { proxy_pass http://127.0.0.1:8901; }
location /     { try_files $uri /index.html; }
# don't cache HTML/version.json (assets are hash-named):
location = /index.html   { add_header Cache-Control "no-store"; }
location = /version.json { add_header Cache-Control "no-store"; }
```

## Login on staging

- **Discord OAuth:** `OAUTH_ALLOWED_OPENER_ORIGINS` in `staging.env` must list the
  exact browser origin(s), e.g. `https://debug.patrykstyla.com` (no trailing
  slash). Defaults to `CORS_ALLOWED_ORIGIN` if unset.
- **Dev login** (skip OAuth) is runtime-gated, works in the release build: set
  `DEV_ACCOUNT_ID` + `DEV_LOGIN_SECRET` in `staging.env` and restart
  `sakiot-staging-web.service`. The frontend shows the button on hosts containing
  `debug`/`dev`/`staging` (`sakiot_stage/src/login/login.tsx`); leave
  `VITE_DEV_LOGIN_SECRET` unset so the secret is prompted, not baked into the
  public bundle.

## GitHub

- `staging` environment holds the same four `DEPLOY_*` secrets as `production`
  (same VPS/user/key); the SSH forced command accepts a `staging <sha>` verb.

See `ops/README.md` for the full deploy/rollback/release-helper docs.

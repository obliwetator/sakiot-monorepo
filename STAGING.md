# Staging

Staging is live on the **same VPS** as production, as a fully separate instance.
The `sakiot` user deploys both, but staging runs as `sakiot-staging` with its own
`sakiot_staging` database role, so it cannot read production's env file, backup
key, data, processes, or database. Pushes to `main` deploy it, except
Markdown/license-only changes. Production ships only on strict `vX.Y.Z` tags.

Feature branches use separate **preview slots** at
`<slot>.preview.patrykstyla.com`, with web + frontend and dev login only.
See [preview instances](PREVIEW.md).

## Pipeline

```
push main ──▶ .github/workflows/deploy-staging.yml ──▶ ssh "staging-ci <sha> [prepare vX.Y.Z]"
          ──▶ ops/deploy stage-ci <sha> [--prepare-production vX.Y.Z] ──▶ build (offline) ▸ migrate ▸ bot+web ▸ frontend
```

A workspace version bump automatically tags and promotes production after
staging verification. For the manual release fallback:

```sh
ops/release vX.Y.Z      # validates clean tree/branch/semver/no-dup + staging matches HEAD, then tags+pushes
```

## Layout (staging vs prod)

| thing            | production            | staging                          |
|------------------|-----------------------|----------------------------------|
| web port         | 8900                  | **8901**                         |
| runtime user     | `sakiot`              | **`sakiot-staging`**             |
| database         | `sakiot_rouvas`       | **`sakiot_staging`** (separate)  |
| database role    | `sakiot`              | **`sakiot_staging`**             |
| Discord bot      | RELEASE bot           | **DEBUG bot**                    |
| web unit         | `sakiot-web.service`  | `sakiot-staging-web.service`     |
| bot unit         | `sakiot-fbi-agent@<id>` | `sakiot-staging-fbi-agent@<id>` |
| data dir         | `/var/lib/sakiot/data` | `/var/lib/sakiot-staging/data`   |
| releases         | `/srv/sakiot`         | `/srv/sakiot-staging`            |
| cache            | `/var/cache/sakiot`   | `/var/cache/sakiot-staging`      |
| env file         | `/etc/sakiot/production.env` | `/etc/sakiot/staging.env`  |
| frontend domain  | patrykstyla.com       | **staging.patrykstyla.com**        |
| frontend docroot | `/var/www/patrykstyla.com` | `/var/www/staging.patrykstyla.com` |

The deploy engine is target-agnostic: the same `ops/sakiot-deploy` Rust binary
serves both targets, driven by the env file plus `SAKIOT_WEB_UNIT` /
`SAKIOT_BOT_UNIT_PREFIX`.

## Setup facts / gotchas (learned during cutover)

- **Builds run offline.** `cargo` runs with `SQLX_OFFLINE=true` and the committed
  `.sqlx` metadata, so a build needs **no live DB**. Without it, sqlx's
  compile-time macros hit `DATABASE_URL` (the empty staging DB) and fail with
  `relation "..." does not exist`. After a `query!` change run
  `scripts/sqlx-prepare.sh` from the repository root and commit `.sqlx`, or the
  offline build errors with `no cached data for this query`.
- **DB backup skipped on staging.** `SAKIOT_SKIP_DB_BACKUP=1` in `staging.env`
  applies migrations without the encrypted pre-migrate backup (staging DB is
  disposable; no `age` key needed). Deployment applies schema migrations; it
  does not run the local development seed. Import fixtures explicitly when
  needed; see [local fixtures](docs/local-fixtures.md).
- **Discord bot token is selected at compile time** (`fbi-agent/src/config.rs`,
  `#[cfg(debug_assertions)]`). A `--release` build reads the `*_RELEASE*` slots,
  so `staging.env` puts the **DEBUG** bot's token/app-id in
  `DISCORD_TOKEN_RELEASE` / `APPLICATION_ID_RELEASE`. Don't run a manual debug
  bot with that token while staging is up (one gateway connection per token).
- **Domain roles:** `staging.patrykstyla.com` serves the deployed staging
  frontend and its API from one origin, the way `patrykstyla.com` serves
  production.
- **Frontend API origin** is baked at build time from `VITE_API_URL` in
  `staging.env` (`https://staging.patrykstyla.com/api/`). Vite reads `VITE_*` from
  the (exported) env. There is **no fallback origin**: the build fails without
  `VITE_API_URL` (`vite.config.ts`). Make sure `production.env` also sets
  `VITE_API_URL`, or the production deploy's frontend build aborts (safely,
  before cutover).
- **Auth cookies are host-only** `__Host-sakiot-*` cookies. Never add a
  `Domain` attribute: parent-domain cookies collide across production,
  staging, and preview hosts.

## nginx

`staging.patrykstyla.com` serves the deployed frontend and staging API:

```nginx
root /var/www/staging.patrykstyla.com;          # NOT /var/www/patrykstyla.com
location /api/ { proxy_pass http://127.0.0.1:8901; }
location = /api/realtime {                      # realtime WebSocket
    proxy_pass http://127.0.0.1:8901;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 75s;
}
location /     { try_files $uri /index.html; }
# don't cache HTML/version.json (assets are hash-named):
location = /index.html   { add_header Cache-Control "no-store"; }
location = /version.json { add_header Cache-Control "no-store"; }
```

Realtime needs the vhost to have the `/api/realtime` location above and the
page CSP to allow the socket; without them, dashboards fall back to polling.
See `ops/README.md`, "Realtime".

## Login on staging

- **Discord OAuth:** `OAUTH_ALLOWED_OPENER_ORIGINS` in `staging.env` must list
  `https://staging.patrykstyla.com`, with no trailing slash, and the debug
  bot's Discord application must allow the redirect
  `https://staging.patrykstyla.com/api/discord_login`. OAuth cookies are
  host-only on `staging.patrykstyla.com`, the same origin as the page.
- **Dev login** (skip OAuth) requires the `dev-login` Cargo feature, which the
  deployer enables for staging and preview release builds. It is also
  runtime-gated: set
  `DEV_ACCOUNT_ID` + `DEV_LOGIN_SECRET` in `staging.env` and restart
  `sakiot-staging-web.service`. The frontend shows the button on hosts containing
  `debug`/`dev`/`staging` (`sakiot-stage/src/login/login.tsx`) and always prompts
  for the secret, so it is never baked into the public bundle.

## GitHub

- `staging` environment holds the same four `DEPLOY_*` secrets as `production`
  (same VPS/user/key); CI uses the `staging-ci <sha>` forced command.
- CI sends its job-scoped, read-only `GITHUB_TOKEN` over SSH stdin for the
  deployer's authenticated Git protocol v2 fetch. No persistent PAT lives on
  the VPS.

See `ops/README.md` for the full deploy/rollback/release-helper docs.

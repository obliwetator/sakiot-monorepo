# Operations

Everything runs on one Debian VPS: production, a staging instance that tracks
`main`, and preview slots for feature branches. Each is a fully separate
instance with its own user, database, data, units and host name. The `sakiot`
user deploys all of them and runs production.

## Environments

| | production | staging | preview slot |
|---|---|---|---|
| deploys on | strict `vX.Y.Z` tag | push to `main` | push to any other branch |
| host | `patrykstyla.com` | `staging.patrykstyla.com` | `<slot>.preview.patrykstyla.com` |
| web port | 8900 | 8901 | `8903 + hash(slot)` |
| Discord bot | RELEASE bot | DEBUG bot | none |
| login | Discord OAuth | Discord OAuth or dev login | dev login only |
| runtime user | `sakiot` | `sakiot-staging` | `sakiot-preview` (shared) |
| database / role | `sakiot_rouvas` / `sakiot` | `sakiot_staging` / `sakiot_staging` | `sakiot_preview_<slot>` / `sakiot_slot` (shared) |
| env file | `/etc/sakiot/production.env` | `/etc/sakiot/staging.env` | `/etc/sakiot/preview.env` (shared) |
| web unit | `sakiot-web.service` | `sakiot-staging-web.service` | `sakiot-preview-<slot>-web.service` |
| bot unit | `sakiot-fbi-agent@<release>` | `sakiot-staging-fbi-agent@<release>` | none |
| data | `/var/lib/sakiot/data` | `/var/lib/sakiot-staging/data` | `/var/lib/sakiot-preview-<slot>/data` |
| releases | `/srv/sakiot` | `/srv/sakiot-staging` | `/srv/sakiot-preview-<slot>` |
| cache | `/var/cache/sakiot` | `/var/cache/sakiot-staging` | `/var/cache/sakiot-preview-<slot>` |
| frontend docroot | `/var/www/patrykstyla.com` | `/var/www/staging.patrykstyla.com` | `/var/www/<slot>.preview.patrykstyla.com` |

The instances share the Cargo target directory, `/var/cache/sakiot/promotions`
and the SQLx test master database. The same `ops/sakiot-deploy` binary serves
every target, driven by the env file plus `SAKIOT_WEB_UNIT` and
`SAKIOT_BOT_UNIT_PREFIX`.

## Release flow

```text
push main ──▶ CI ──▶ deploy-staging.yml ──▶ ssh "staging-ci <sha> [prepare vX.Y.Z]"
          ──▶ ops/deploy stage-ci ──▶ build ▸ migrate ▸ bot + web handoff ▸ frontend
```

Pushes that change only Markdown or `LICENSE` skip CI and every deploy. CI runs
each suite affected since the last commit a successful run of the same workflow
verified on that branch (`scripts/ci-baseline.sh`), so a push whose run failed
or was superseded is still covered. It runs everything when no such commit
exists, and for a version bump.

`[workspace.package] version` in the root `Cargo.toml` is the only release
trigger:

1. Bump the version in a PR (`cargo check` updates `Cargo.lock` with it).
2. Merge to `main`. The `release-candidate` job compares the version with the
   latest release tag. A higher version makes CI run every suite and makes the
   staging deploy also build production bot, web and frontend variants, hash
   them, and publish them under `/var/cache/sakiot/promotions` once staging is
   healthy. A lower version fails the job, which blocks the staging deploy.
3. `auto-tag` checks that staging serves this exact commit, tags `v<version>`,
   and dispatches `deploy-release.yml` on the tag. The dispatch is needed
   because a tag pushed with the workflow's own `GITHUB_TOKEN` does not trigger
   the tag-push event; no personal access token is involved.
4. The release run requires the promotion for that exact tag and SHA,
   re-verifies every digest, takes an encrypted backup, migrates, deploys the
   changed components and records a manifest only after health checks pass. A
   missing or modified promotion fails before migrations or service changes.

**Never `git revert` a version bump.** The version would drop below the latest
tag and block every staging deploy until it rises again. Roll forward with a
new version, and undo a bad release with the rollback workflow.

The manual fallback validates the tree, branch, sync state, semver, tag
uniqueness, and that staging serves the commit (`--skip-staging-check` only
when justified):

```sh
ops/release v1.2.3
```

A raw `git tag -a v1.2.3 && git push origin v1.2.3` also works, without those
checks; it runs full CI and builds on the VPS when no promotion exists. Only
strict `vX.Y.Z` tags deploy: `v1.23` or `v1.2.3-rc1` fail input validation
before the SSH step. The deployer also rejects moved or previously successful
tags, and records documentation-only tags as no-ops.

### Rollback

Run the `Roll back production` workflow with a prior tag. It reuses the
binaries and frontend built for that commit when its release directory still
exists under `/srv/sakiot/releases`, and rebuilds any component it cannot
reuse (`SAKIOT_ROLLBACK_FORCE_REBUILD=1` rebuilds everything). Rollback never
reverses migrations or restores a database, and it refuses when the migration
files differ from production's unless `allow_schema_mismatch` is chosen after a
compatibility review.

## VPS bootstrap

Install Git, Rust, Bun, `protoc`, OpenSSL development headers, FFmpeg,
`audiowaveform`, PostgreSQL client tools, SQLx CLI, `age`, `rclone`, `rsync`,
`sudo`, `flock`, `jq` and Python 3. `grpcurl` helps with manual bot
diagnostics. The frontend build compiles `sakiot-dsp` to WASM, so the `sakiot`
user also needs the pinned `wasm-bindgen` CLI. Re-run this whenever
`sakiot-dsp/wasm-bindgen-cli-version` changes; `scripts/build-dsp.sh` fails on
a mismatch:

```sh
sudo -u sakiot bash -lc 'cd /path/to/sakiot-monorepo \
  && rustup target add wasm32-unknown-unknown \
  && cargo install wasm-bindgen-cli --version "$(cat sakiot-dsp/wasm-bindgen-cli-version)" --locked'
```

The manual `release` and `staging` verbs, which CI has not verified, run the Rust
tests on the VPS. Give them a dedicated test role and master database, and set
`SAKIOT_TEST_DATABASE_URL` in both runtime env files. Its database name must
end in `_test` and differ from the runtime database:

```sql
CREATE ROLE sakiot_test LOGIN CREATEDB PASSWORD 'replace_me';
CREATE DATABASE sakiot_test OWNER sakiot_test;
```

As root:

```sh
ops/install-production.sh /root/github-deploy-key.pub
$EDITOR /etc/sakiot/production.env
$EDITOR /etc/sakiot/staging.env
sudo -u postgres psql   # roles and databases, below
systemctl enable sakiot-web.service
```

Each environment's role is the only one that may connect to its database. Use
the passwords from each env file's `DATABASE_URL`:

```sql
CREATE ROLE sakiot_staging LOGIN PASSWORD 'replace_me';
CREATE DATABASE sakiot_staging OWNER sakiot_staging;
REVOKE CONNECT, TEMPORARY ON DATABASE sakiot_staging FROM PUBLIC;
REVOKE CONNECT, TEMPORARY ON DATABASE sakiot_rouvas FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE sakiot_rouvas TO sakiot;
```

Preview slots create their shared `sakiot_slot` role themselves.

The installer creates the `sakiot`, `sakiot-staging` and `sakiot-preview`
users, the production and staging directories, the systemd units, a restricted
`authorized_keys` line, and a root-owned systemd command validator behind
narrow sudo rules. Check that the key line contains `restrict` and the forced
command. It also installs the backup scripts under
`/usr/local/lib/sakiot-deploy/backup` and the backup and restore-test timers
(see [Database backups](#database-backups)).

`ops/` is installed out of band at `/usr/local/lib/sakiot-deploy`, so a release
tag cannot change the root-owned SSH bootstrap. After reviewing changes to
`ops/`, refresh it with `sudo ops/update-deploy-engine.sh`.

## GitHub

Create `production` and `staging` environments, without required reviewers,
each holding:

- `DEPLOY_HOST`
- `DEPLOY_USER` (`sakiot`)
- `DEPLOY_SSH_KEY`
- `DEPLOY_KNOWN_HOSTS` (a pre-verified host key, not live `ssh-keyscan` output)

Preview deploys reuse the `staging` secrets. Every workflow reaches the VPS
through the restricted key's forced command (`ops/ssh/forced-command`), which
accepts `release`, `release-ci`, `release-promoted`, `rollback`, `staging`,
`staging-ci`, `preview-ci`, `preview-up` and `preview-remove`.

No token is stored on the VPS. Deploy workflows send their job-scoped,
read-only `GITHUB_TOKEN` over SSH stdin; it authenticates a Git protocol v2
fetch through an ephemeral askpass file and is deleted when the fetch ends.
Keep `permissions.contents` at `read` (only the staging auto-tag job has
write). Protect the owner account with a passkey or 2FA, and never give
pull-request workflows deployment secrets or use `pull_request_target`.

## Deploy engine

`ops/deploy`, the forced command's entry point, dispatches to the Rust engine
in `ops/sakiot-deploy/`. It takes an exclusive `deploy.lock` per target or
slot, so different instances deploy concurrently, and a second lock serializes
builds in the shared Cargo target directory.

The engine binary is root-owned at
`/usr/local/lib/sakiot-deploy/bin/sakiot-deploy`, built by the installers from
the checkout and never from a release worktree, so a broken commit cannot
break deploys. The installers stamp the `ops/sakiot-deploy` tree they built in
`/usr/local/lib/sakiot-deploy/engine-src-tree`. A deploy whose commit carries
newer engine changes prints a `WARNING:` and still runs on the installed
engine; refresh it with `sudo ops/update-deploy-engine.sh`.

The `*-ci` verbs are reachable only after the CI job succeeds, and skip the VPS
test pass. The bash suites for the out-of-band shims (`ops/tests/run.sh`:
forced command, systemctl wrapper, frontend publish) run in CI and on the VPS
through `update-deploy-engine.sh`.

`sakiot-deploy --dry-run {release|rollback|stage} ...` (local only, not
reachable over SSH) reports component selection and reuse, then stops before
any build, migration or service change. Check an instance with
`sudo -u sakiot /usr/local/lib/sakiot-deploy/bin/sakiot-deploy status {production|staging|preview <slot>}`.

## Staging

Staging reuses the production engine through `ops/deploy stage <sha>`: it
builds, migrates `sakiot_staging`, performs the same drain-aware bot handoff
and health-gated web cutover, and prunes old staging releases.

- **Bot token.** `fbi-agent` picks its Discord credentials by build profile
  (`#[cfg(debug_assertions)]` in `fbi-agent/src/config.rs`), and staging runs a
  release build, so `staging.env` puts the DEBUG bot's token and application
  id in `DISCORD_TOKEN_RELEASE` and `APPLICATION_ID_RELEASE`. Don't run a local
  debug bot with that token while staging is up: Discord allows one gateway
  connection per token.
- **Migrations without backup.** `SAKIOT_SKIP_DB_BACKUP=1` in `staging.env`
  skips the encrypted pre-migrate backup. Deploys never run the development
  seed; import data with [`cargo dev fixtures`](../docs/local-fixtures.md).
- **Offline builds.** `cargo` builds with `SQLX_OFFLINE=true` and the committed
  `.sqlx`, so a stale `.sqlx` fails the build with `no cached data for this
  query`. Run `scripts/sqlx-prepare.sh` and commit it.
- **API origin.** The frontend's `VITE_API_URL` is baked in at build time from
  the env file, with no fallback. Production and staging each serve the
  frontend and its API from one origin.
- **Cookies.** Auth cookies are host-only `__Host-sakiot-*` cookies. Never add
  a `Domain` attribute: parent-domain cookies would collide across production,
  staging and preview hosts.
- **Login.** Discord OAuth needs `https://staging.patrykstyla.com` (no trailing
  slash) in `OAUTH_ALLOWED_OPENER_ORIGINS` and the redirect
  `https://staging.patrykstyla.com/api/discord_login` on the debug bot's
  application. Dev login needs the `dev-login` Cargo feature, which the
  deployer enables for staging and preview builds only, plus `DEV_ACCOUNT_ID`
  and `DEV_LOGIN_SECRET` in the env file. The frontend shows its button on
  hosts containing `debug`, `dev`, `staging` or `preview` and always prompts
  for the secret, so the secret never reaches the bundle.

## Preview slots

A preview slot runs the web server and frontend for one branch, with no
Discord bot and dev login only, so several branches can be tested side by side
without touching staging. Bot behavior is exercised on staging.

### Automatic lifecycle

The `Deploy preview` workflow handles every pushed branch other than `main`
(docs-only pushes excepted). It derives a slot from the branch name,
provisions it if missing (`preview-up`), deploys the commit after CI
(`preview-ci <slot> <sha>`), and tears the slot down when the branch is deleted
(`preview-remove`). A branch that exists only locally deploys nothing.
Deploys to different slots run in parallel.

The slot name is the branch lowercased, with each run of characters outside
`[a-z0-9]` replaced by `-`, trimmed and cut to 32 characters
(`feature/Clip-Editor v2` → `feature-clip-editor-v2`). `feature/foo` and
`feature_foo` therefore share a slot, and the later push wins it.

To deploy by hand, run **Actions → Deploy preview** from the branch ("Use
workflow from") and enter the slot. It tests and deploys that branch's head,
provisioning the slot if needed.

### One-time setup

```sh
# As root on the VPS, after any ops/ change:
ops/update-deploy-engine.sh
# The shared env file, created only if absent:
test -e /etc/sakiot/preview.env || install -o root -g sakiot -m 0640 ops/preview.env.example /etc/sakiot/preview.env
$EDITOR /etc/sakiot/preview.env
```

Set the shared credentials once: `CLOUDFLARE_API_TOKEN` for DNS records,
`DEV_ACCOUNT_ID` and `DEV_LOGIN_SECRET`, `CERTBOT_EMAIL` for slot
certificates, and the JWT, registry and database secrets. The Discord OAuth
values only have to be non-empty. Everything host-specific (`VITE_API_URL`,
CORS and opener origins) is derived per slot. The media archive is disabled in
the example; enable `SAKIOT_MEDIA_ARCHIVE_ENABLED` when a branch needs it.

### What `ops/preview-slot.sh` does

The workflow runs it as root through a narrow sudo rule
(`ops/sudoers/sakiot-deploy`). It is root-owned and validates the slot name,
so it can only create or remove that slot's resources. Run it by hand to
provision a slot that is not tied to a branch: `ops/preview-slot.sh <slot>`,
and `--remove` to tear it down.

- **DNS.** It creates the `A` record `<slot>.preview.patrykstyla.com` through
  the Cloudflare API. With one wildcard record `*.preview.patrykstyla.com`
  instead, pass `--no-dns`. Certbot issues each slot's certificate over
  HTTP-01 either way.
- **Users and directories.** It creates the `sakiot-preview` user and
  `sakiot_slot` role once, then the slot's data, deploy, cache, release and
  web root directories.
- **Database.** On first creation it copies the staging database and media,
  so staging needs a migrated schema first. Re-provisioning keeps slot-local
  data, and every run seeds the dev-login account.
- **Migrations.** Every deploy applies the branch's pending migrations as
  `sakiot_slot`, without a backup, with `--ignore-missing` because the staging
  copy may hold migrations the branch has not merged. Editing a migration
  after a slot applied it fails the next deploy on its checksum; recreate the
  slot (`preview-remove`, then `preview-up`) or restore the migration.
- **Unit and vhost.** It installs the web unit from the staging template and
  the vhost from `ops/nginx/preview-slot.conf.example`, validates nginx before
  reloading, and runs `certbot --nginx` when `CERTBOT_EMAIL` is set. The vhost
  is rendered once, at provisioning, so re-render older slots after the
  template changes.
- **Removal.** `--remove` deletes the slot's DNS record, certificate, vhost,
  unit, database and local media copies. With a delete-capable key in the
  root-only `/etc/sakiot/preview-b2-purge.env` (`B2_PURGE_KEY_ID`,
  `B2_PURGE_KEY_SECRET`), it also hides the B2 objects the slot created: keys in
  the slot's `media_objects` that staging's lacks. Retained versions stay
  recoverable. Without the key, the B2 purge is skipped with a warning.

A `preview-ci` deploy loads the shared `preview.env` and rewrites these tokens
per slot. It writes the resulting port, origins and `DISCORD_REDIRECT_URI` into
the release's `web/service.env`, which the unit reads after the shared file:

| in `preview.env` | per slot |
|---|---|
| `PORT` (8903) | `8903 + hash(slot)` |
| `sakiot_preview` | `sakiot_preview_<slot>` |
| `sakiot-preview` | `sakiot-preview-<slot>` |
| `preview.patrykstyla.com` | `<slot>.preview.patrykstyla.com` |

That rewrite is why the shared role is called `sakiot_slot`: a role name
containing `sakiot_preview` would turn into a per-slot role that does not
exist.

## nginx and realtime

Production and staging vhosts are not in the repository; preview slots use
`ops/nginx/preview-slot.conf.example`. Each vhost serves the frontend docroot,
proxies `/api/` to the instance's port, and forwards WebSocket upgrades for
the realtime socket:

```nginx
location /api/ { proxy_pass http://127.0.0.1:<port>; }
location = /api/realtime {
    proxy_pass http://127.0.0.1:<port>;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 75s;                 # the server heartbeats every 20 s
}
location /     { try_files $uri /index.html; }
# Assets are hash-named; never cache the entry points.
location = /index.html   { add_header Cache-Control "no-store"; }
location = /version.json { add_header Cache-Control "no-store"; }
```

The `map $http_upgrade $connection_upgrade` form belongs in the `http` context,
not the vhost. The page's CSP `connect-src` must allow `wss://<api host>` (or
`wss:`, as the preview template does); check with
`curl -sI https://<host>/ | grep -i content-security-policy`. The server
accepts the socket only from the exact origins in `CORS_ALLOWED_ORIGIN` and
`OAUTH_ALLOWED_OPENER_ORIGINS`. Without the location or the CSP, dashboards
fall back to polling. After `nginx -t && systemctl reload nginx`, the network
panel should show `/api/realtime` as `101 Switching Protocols`, and
`realtime_listener_connected` should be 1.

Watch `pg_notification_queue_usage` (alert well below 1: a full NOTIFY queue
fails every notifying commit, recordings included), `realtime_resyncs` and
`realtime_queue_overflows`.

## Environment isolation

Staging and preview slots run code production has not shipped, and previews
are reachable by anyone with the URL, so neither may reach production.

- **Files.** The env files and the backup key stay `root:sakiot`; systemd
  reads `EnvironmentFile=` as root, so runtime users never need them.
- **Data and releases.** Each runtime user owns its data directory. The setgid
  bit keeps new files in its group, which `sakiot` joins to deploy and import
  fixtures, and every unit sets `UMask=0007`. Releases are reachable through
  the group of `/srv/<instance>/releases` and `current`.
- **Database.** Each role owns its database and `CONNECT` is revoked from
  `PUBLIC`. PostgreSQL also listens on the observability Docker network's
  gateway (`172.18.0.1`, `conf.d/listen.conf`), and `pg_hba.conf` admits that
  network for `sakiot_rouvas` only, where Grafana reads as the read-only
  `grafana_ro` role.
- **Sandbox.** Every runtime unit uses `ProtectSystem=strict` with the data
  directory as the only writable path, a system-call filter, no capabilities
  and kernel-surface restrictions.
- **Processes.** Another user cannot read a service's `/proc/<pid>/environ`,
  where its database password lives.
- **Loopback.** Every instance listens on `127.0.0.1`, which any local user can
  reach, so internal endpoints authenticate the caller instead: the agent's
  gRPC services and web-server's `/internal/fbi-agent/grpc-endpoints` require
  the env file's `FBI_AGENT_REGISTRY_SECRET`. Give each env file its own value;
  a shared secret would let staging or preview code call production. Release
  builds of the agent refuse to start without one.

To rotate a role's password, `ALTER ROLE <role> PASSWORD '...'` as `postgres`,
update `DATABASE_URL` and `BACKUP_DATABASE_URL` (and `PGUSER`/`PGPASSWORD` if
set) in the env file, and restart that instance's web server and bot.

### Sandbox denials

Nothing logs a sandbox denial as such: the forbidden call fails and the
service logs the error like any other. Search the journal for the error texts:

```sh
journalctl -u 'sakiot-*' --since -1d | grep -iE 'denied|not permitted|EACCES|read-only file system'
```

| Error in the log | Cause |
|---|---|
| `Read-only file system (os error 30)` | A write outside the data directory (`ProtectSystem=strict`). |
| `Permission denied (os error 13)` | Another user's file, such as another environment's env file or data, or anything under `/home` (`ProtectHome`). |
| `Operation not permitted (os error 1)` | A system call outside `SystemCallFilter=@system-service`, or a privileged operation. |
| `Address family not supported by protocol` | A socket type outside `RestrictAddressFamilies`. |
| `No such file or directory` for a device | A device outside the minimal `/dev` (`PrivateDevices`). |

PostgreSQL's `permission denied for <object>` matches too, but it is a
database privilege. Rare code paths (archiving, pruning, crash recovery,
unusual ffmpeg inputs) fail only when they first run, so check again a day
after a unit or path changes.

To allow what the service tried, change the unit in `ops/systemd/`: add a
writable path to `ReadWritePaths=`, or better use `StateDirectory=` or
`CacheDirectory=`. Otherwise fix the code. To reproduce, run the command in a
throwaway sandbox with the same settings (add `strace -f` to see the failing
call):

```sh
sudo systemd-run --pty -p User=sakiot -p ProtectSystem=strict \
  -p ReadWritePaths=/var/lib/sakiot/data <command>
systemd-analyze security sakiot-web.service
```

## Health, alerts and headers

`/livez` is a dependency-free process check; point restart checks at it.
`/readyz` and `/healthz`, used for routing and deploy checks, return 503 when:

- PostgreSQL does not answer within two seconds;
- a media, composition, archive or recording-deletion worker has exited;
- a media, composition or permanent-deletion lease stays expired for two
  minutes, or a permanent deletion has exhausted its retries;
- the oldest *due* queued job, or due archive work when B2 is enabled, is over
  an hour old. Scheduled retries count only from their retry time.

A missing or conflicted archive object is reported without making the
instance unready, and paused deletion jobs wait for operator review. The
response lists `failed_workers` and `queues`; read it before restarting.

The web process records OTLP gauges every 30 seconds: `web_server_ready`,
`media_worker_failed`, and per queue `media_queue_queued_jobs`,
`media_queue_active_processes` and `media_queue_oldest_age_seconds` (the
deletion queue is labelled `recording_deletion`). Alert on
`web_server_ready == 0`, `media_worker_failed > 0`, or queue age approaching
the readiness threshold.

The API sets CSP, HSTS, frame, MIME, referrer and permissions headers on every
response, including errors, OAuth, media and CORS preflights. The OAuth
callback uses a single-use script nonce, and Scalar's CDN exception is limited
to `/scalar`. Static SPA responses get their headers from nginx (the preview
template shows the policy); never apply that CSP to `/api/`, where it would
block the OAuth nonce script.

## Database backups

Production backups run as `sakiot` into `/var/lib/sakiot/backups` and are
copied to B2. Configure `BACKUP_DATABASE_URL`, `BACKUP_DIR`, `AGE_RECIPIENT`
and `AGE_KEY_FILE` in `production.env`, and install the existing age private
key at `/etc/sakiot/age-key.txt` as `root:sakiot`, mode `0640`; the installer
never generates or replaces keys. Enable the timers once configured:

```sh
systemctl enable --now sakiot-db-backup-hourly.timer \
  sakiot-db-backup-nightly.timer sakiot-db-restore-test.timer
systemctl list-timers 'sakiot-db-*'
journalctl -u 'sakiot-db-backup@*' -u sakiot-db-restore-test.service
```

What the scripts do, and how to restore, is in
[`sakiot-db/ops/backup/README.md`](../sakiot-db/ops/backup/README.md). The
media archive has its own [runbook](B2_MEDIA_ARCHIVE.md).

## Runtime state

```text
/etc/sakiot/production.env
/var/lib/sakiot/data
/var/lib/sakiot/deploy
/var/lib/sakiot/backups
/srv/sakiot/releases
/srv/sakiot/current
/var/cache/sakiot
/var/cache/sakiot/promotions
```

Release manifests are at `/srv/sakiot/releases/<release>/manifest.json`, and
`/var/lib/sakiot/deploy/current.manifest` points to the last successful one.
Stopped releases are kept on purpose. Never remove a release directory while
its `sakiot-fbi-agent@...` unit is active or draining.

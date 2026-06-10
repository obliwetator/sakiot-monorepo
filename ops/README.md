# Production deployment

Production deploys run from GitHub-hosted Actions runners when a new `v*` tag
is pushed. The runner has read-only repository permission and sends only the
tag and commit SHA through a forced SSH command. Builds, tests, backups,
migrations, service changes, and health checks run on the VPS.

## VPS bootstrap

Install required tools: Git, Rust, Bun, `protoc`, OpenSSL development headers,
FFmpeg, `audiowaveform`, PostgreSQL client tools, SQLx CLI, `age`, `grpcurl`,
`jq`, `rsync`, Python 3, `flock`, and `sudo`.

Create a dedicated SQLx test role and master database. Deploy-time Rust tests
create and remove a temporary database per test; they never use the runtime
`DATABASE_URL`.

```sql
CREATE ROLE sakiot_test LOGIN CREATEDB PASSWORD 'replace_me';
CREATE DATABASE sakiot_test OWNER sakiot_test;
```

Set `SAKIOT_TEST_DATABASE_URL` in both runtime env files. Its database name must
end in `_test` and differ from the runtime database. Keep this role unprivileged
on production and staging databases.

As root:

```sh
ops/install-production.sh /root/github-deploy-key.pub
$EDITOR /etc/sakiot/production.env
$EDITOR /etc/sakiot/staging.env
createdb sakiot_staging
systemctl enable sakiot-web.service
```

The installer creates the `sakiot` user, persistent directories for both the
production and staging instances, systemd units, restricted `authorized_keys`,
and a root-owned systemd command validator behind narrowly scoped sudo rules. It
also installs production backup scripts under
`/usr/local/lib/sakiot-deploy/backup`, creates `/var/lib/sakiot/backups`, and
enables hourly, nightly, and monthly restore-test timers.
Validate the generated key line contains `restrict` and the forced command. Keep
manual debug services under the developer account; production units are named
`sakiot-web.service` and `sakiot-fbi-agent@<release>.service`, and the staging
instance uses `sakiot-staging-web.service` and
`sakiot-staging-fbi-agent@<release>.service`.

For the first release on a host still running the prior `tulipan` user units,
set `SAKIOT_LEGACY_BOT_UNIT`, `SAKIOT_LEGACY_BOT_GRPC`, and
`SAKIOT_LEGACY_WEB_ENABLED=1`. The deployer drains the old bot, stops the old
web service only after builds pass, and restores the old web service if new
health checks fail. These settings are ignored after production state exists.

Copy this repository's `ops` directory to `/usr/local/lib/sakiot-deploy` after
reviewing deployment-framework changes. Application release tags cannot modify
the root-owned SSH bootstrap by themselves.

## Database backups

Production backups belong to the `sakiot` service account and are stored in
`/var/lib/sakiot/backups`. Configure `BACKUP_DATABASE_URL`, `BACKUP_DIR`,
`AGE_RECIPIENT`, and `AGE_KEY_FILE` in `/etc/sakiot/production.env`. Install the
existing age private key at `/etc/sakiot/age-key.txt` with owner `root:sakiot`
and mode `0640`; the installer does not generate or replace keys. Fresh hosts
with placeholder backup settings receive the units but must enable them after
configuration:

```sh
systemctl enable --now sakiot-db-backup-hourly.timer \
  sakiot-db-backup-nightly.timer sakiot-db-restore-test.timer
systemctl list-timers 'sakiot-db-*'
journalctl -u 'sakiot-db-backup@*' -u sakiot-db-restore-test.service
```

Do not keep a second cron schedule after the timers are active. Copy historical
encrypted dumps into `/var/lib/sakiot/backups`, verify a backup and restore test,
then remove the old cron block.

## GitHub environment

Create a `production` environment without required reviewers and add:

- `DEPLOY_HOST`
- `DEPLOY_USER` (`sakiot`)
- `DEPLOY_SSH_KEY`
- `DEPLOY_KNOWN_HOSTS` (pre-verified host key, not live `ssh-keyscan` output)

Create a `staging` environment with the same four secrets (same VPS, same deploy
user and key). The staging deploy reuses the restricted key; the forced command
accepts a `staging <sha>` verb in addition to `release`/`rollback`.

Repository owner account should use a passkey or 2FA. Do not add deployment
secrets to pull-request workflows or use `pull_request_target`.

## Staging

Every push to `main` deploys to the staging instance via the
`Deploy staging` workflow (`staging <sha>` over the restricted SSH). Staging runs
on the same VPS as a fully separate instance: the `sakiot_staging` database, port
`8901`, the DEBUG Discord bot, `/var/lib/sakiot-staging` + `/srv/sakiot-staging`,
its own systemd units, and `debug.patrykstyla.com` for the frontend. Its runtime
profile lives in `/etc/sakiot/staging.env`.

Staging reuses the production deploy engine through `ops/deploy stage <sha>`: it
builds, runs `sakiot_staging` migrations, performs the same drain-aware bot
handoff and health-gated web cutover, and prunes old staging releases — without
touching production. Because the bot binary is built with `cargo build --release`
(which reads the `*_RELEASE*` credential slots, see `FBI-agent/src/config.rs`),
`staging.env` puts the DEBUG bot's token/application id in those slots.

## Release

Cut a production release with the helper, which validates before it pushes:

```sh
ops/release v1.2.3
```

It refuses a dirty tree, a non-`main` branch, a local/remote that is out of sync,
a non-strict-semver or already-existing tag, and a commit that has not yet been
deployed and verified on staging (`debug.patrykstyla.com/version.json`). Override
the staging check only when justified with `--skip-staging-check`.

The raw equivalent (no safety checks) is still:

```sh
git tag -a v1.2.3 -m "v1.2.3"
git push origin v1.2.3
```

Production deploys only on **strict semver** tags `vX.Y.Z`; a typo like `v1.23`
or a suffix like `v1.2.3-rc1` matches neither workflow and is a safe no-op.

The deployer rejects invalid, moved, or previously successful tags. It locks
deployment state, verifies the tag commit, selects changed components, completes
all builds/tests before mutations, runs encrypted backup before migrations,
deploys bot/web/frontend, and records a manifest only after health checks pass.
Documentation-only tags are recorded as successful no-ops.

## Rollback

Run the `Roll back production` workflow with a prior tag. Rollback reuses the
binaries and frontend `dist` already built for that commit when its release
directory still exists under `/srv/sakiot/releases` (kept by retention), copying
them into the new rollback release instead of recompiling; any component without
a reusable artifact is rebuilt from source. Set `SAKIOT_ROLLBACK_FORCE_REBUILD=1`
to skip reuse and rebuild everything. Rollback never reverses migrations or
restores a database, and blocks when migration files differ from current
production unless `allow_schema_mismatch` is explicitly selected after
compatibility review.

## Runtime state

```text
/etc/sakiot/production.env
/var/lib/sakiot/data
/var/lib/sakiot/deploy
/var/lib/sakiot/backups
/srv/sakiot/releases
/srv/sakiot/current
/var/cache/sakiot
```

Release manifests are under `/srv/sakiot/releases/<release>/manifest.json`;
`/var/lib/sakiot/deploy/current.manifest` points to the last successful one.
Stopped releases are intentionally retained. Never remove a release directory
while its `sakiot-fbi-agent@...` unit is active or draining.

## Temporary legacy data

If production was cut over before the recording tree was migrated, keep the
canonical production path while bind-mounting the existing tree:

```sh
sudo ./ops/use-legacy-data.sh /home/tulipan/projects/sakiot/data
```

The script stops the production bot and web server, merges files created since
cutover into the legacy tree, grants the `sakiot` account access with POSIX
ACLs, adds an idempotent `/etc/fstab` bind entry, mounts the tree at
`/var/lib/sakiot/data`, and restarts both services. It does not copy the full
recording archive or change `DATABASE_URL`.

Remove the bind entry only after the legacy tree has been copied into an
independent production filesystem while both services are stopped.

Permanent migration procedure: [DATA_MIGRATION_PLAN.md](DATA_MIGRATION_PLAN.md).

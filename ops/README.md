# Production deployment

Production deploys run from GitHub-hosted Actions runners when a new `v*` tag
is pushed. The runner has read-only repository permission and sends only the
tag and commit SHA through a forced SSH command. Builds, tests, backups,
migrations, service changes, and health checks run on the VPS.

## VPS bootstrap

Install required tools: Git, Rust, Bun, `protoc`, OpenSSL development headers,
FFmpeg, `audiowaveform`, PostgreSQL client tools, SQLx CLI, `age`, `rsync`,
and `sudo`. The bash deploy engine additionally needs `grpcurl`, `jq`,
Python 3, and `flock`; the Rust engine does that work in-process.

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
reviewing deployment-framework changes (`ops/update-deploy-engine.sh` does this
plus the engine build below). Application release tags cannot modify the
root-owned SSH bootstrap by themselves.

## Deploy engine

`ops/deploy` (the SSH forced-command entry point) dispatches to the Rust
deploy engine in `ops/sakiot-deploy/`. It originated as a behavior-identical
port of a bash engine that has since been deleted: env vars, state files,
`manifest.json` schema, and release layout are unchanged, so releases made
by the old engine remain valid rollback targets. The engine takes an
exclusive `deploy.lock`, so deploys can never interleave.

The binary is installed out-of-band like the rest of `ops/`:
`install-production.sh` (and `update-deploy-engine.sh` for later refreshes)
builds `--package sakiot-deploy` from the checkout as the `sakiot` user and
installs root-owned `/usr/local/lib/sakiot-deploy/bin/sakiot-deploy`. It is
never built from the release worktree, so a broken commit cannot brick
deploys. Engine tests run in CI (`cargo test --workspace`) and on the VPS
during the deploy-time workspace test step.

`sakiot-deploy --dry-run {release|rollback|stage} ...` (local only, not
reachable through the SSH forced command) reports component selection and
reuse decisions, then stops before any build, migration, or service change.

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
`Deploy staging` workflow (`staging <sha>` over the restricted SSH). Docs-only
pushes (`*.md`, `LICENSE`) skip CI and the staging deploy entirely via
`paths-ignore` on the workflow trigger. Staging runs
on the same VPS as a fully separate instance: the `sakiot_staging` database, port
`8901`, the DEBUG Discord bot, `/var/lib/sakiot-staging` + `/srv/sakiot-staging`,
its own systemd units, and `staging.patrykstyla.com` for the frontend. Its runtime
profile lives in `/etc/sakiot/staging.env`.

Staging reuses the production deploy engine through `ops/deploy stage <sha>`: it
builds, runs `sakiot_staging` migrations, performs the same drain-aware bot
handoff and health-gated web cutover, and prunes old staging releases — without
touching production. Because the bot binary is built with `cargo build --release`
(which reads the `*_RELEASE*` credential slots, see `FBI-agent/src/config.rs`),
`staging.env` puts the DEBUG bot's token/application id in those slots.

## Release

The normal path is version-bump driven. The workspace version in the root
`Cargo.toml` (`[workspace.package] version`, inherited by every crate) is the
single source of truth:

1. Bump the version in a PR (e.g. `1.0.6` → `1.0.7`; remember `Cargo.lock`
   updates with it — run `cargo check`).
2. Merge to `main`. CI deploys staging as usual.
3. The `auto-tag` job in `deploy-staging.yml` then compares the workspace
   version against the latest `v*` tag. If it is strictly higher (strict semver
   only) and staging is verified to be serving this exact commit, it tags
   `v<version>`, pushes the tag, and dispatches `deploy-release.yml` on it.
   The explicit dispatch is needed because a tag pushed with the workflow's
   own `GITHUB_TOKEN` does not trigger the tag-push event (GitHub's recursion
   guard); `workflow_dispatch` is exempt. No personal access token is
   involved, so the release path is not tied to any individual account.
4. The dispatched run validates the tag and deploys production exactly as a
   manually pushed tag would.

Merges that do not bump the version deploy staging only; the `auto-tag` job is
a no-op. A version lower than the latest release fails the job loudly.

**Never `git revert` a commit that bumped the version** (watch for this when
reverting a feature PR that included a bump). The workspace version would drop
below the latest release tag, and the `auto-tag` job then fails on every merge
to `main` until the version is raised again. Staging still deploys, but CI
stays red. Always roll forward instead: new commit, higher version. To undo a
bad release in production, use the rollback workflow — not a revert of the
version bump.

The manual fallback still works — cut a release with the helper, which
validates before it pushes:

```sh
ops/release v1.2.3
```

It refuses a dirty tree, a non-`main` branch, a local/remote that is out of sync,
a non-strict-semver or already-existing tag, and a commit that has not yet been
deployed and verified on staging (`staging.patrykstyla.com/version.json`). Override
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

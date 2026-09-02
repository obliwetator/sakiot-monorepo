# Production deployment

Production deploys run from GitHub-hosted Actions runners when a new `v*` tag
is pushed. The runner has read-only repository permission and sends the tag,
commit SHA, and its short-lived repository token through a forced SSH command.
The token travels on SSH stdin, authenticates Git protocol v2 through an
ephemeral askpass file, and is deleted when the source fetch completes (or when
the deploy exits after a fetch failure); it is never stored in a repository URL
or persistent credential store. CI runs tests once. A
version-bump staging deploy builds and hashes the production bundle on the
Debian VPS; production promotes that exact bundle, then performs backups,
migrations, service changes, and health checks.

## VPS bootstrap

Install required tools: Git, Rust, Bun, `protoc`, OpenSSL development headers,
FFmpeg, `audiowaveform`, PostgreSQL client tools, SQLx CLI, `age`, `rclone`, `rsync`,
and `sudo`. The bash deploy engine additionally needs `grpcurl`, `jq`,
Python 3, and `flock`; the Rust engine does that work in-process.

Create a dedicated SQLx test role and master database. Legacy/local deploy
verbs retain deploy-time Rust tests as a safe fallback; those tests create and
remove a temporary database per test and never use runtime `DATABASE_URL`.

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
deploys. Engine tests run in CI (`cargo test --workspace`). Authenticated `*-ci`
forced-command verbs are reachable only after the Actions test job succeeds.
They receive that job's read-only `GITHUB_TOKEN` on stdin, force authenticated
Git protocol v2 for the source fetch, and skip the duplicate VPS test pass.
Legacy/local verbs still test on the VPS. The bash suites for the
out-of-band shims (`ops/tests/run.sh`: forced command, systemctl wrapper,
frontend publish) run in CI on every PR and on the VPS via
`update-deploy-engine.sh`.

Both installers also stamp `/usr/local/lib/sakiot-deploy/engine-src-tree`
with the `ops/sakiot-deploy` tree OID they built from (skipped with a warning
when the checkout is dirty). Every deploy compares that stamp against the
release commit and prints a `WARNING:` when the release carries engine
changes the installed binary predates — the deploy still runs, on the old
engine. When you see it, refresh with `sudo ops/update-deploy-engine.sh`.

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

No separate GitHub PAT is installed on the VPS. Deploy workflows forward their
automatic, job-scoped `GITHUB_TOKEN` over SSH stdin. Keep workflow
`permissions.contents` at `read` (the staging auto-tag job has its separate,
explicit write permission).

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
(which reads the `*_RELEASE*` credential slots, see `fbi-agent/src/config.rs`),
`staging.env` puts the DEBUG bot's token/application id in those slots.

CI uses `stage-ci`; when the workspace version exceeds the latest release tag,
it also supplies `--prepare-production vX.Y.Z`. The deployer builds production
bot, web, and frontend variants before any staging mutation, records SHA-256
digests, and atomically publishes them under
`/var/cache/sakiot/promotions`. Publication happens only after staging health
checks and state recording succeed.

## Preview

A third (or more) instance model for deploying feature branches without
touching the main-tracking staging instance. The `Deploy preview` workflow
provisions a slot per pushed non-`main` branch automatically (`preview-up`
runs `ops/preview-slot.sh` as root via a narrow sudo rule), deploys the pushed
commit with `preview-ci <slot> <sha>` after CI, and tears the slot down
(`preview-remove`) when the branch is deleted. Manual `workflow_dispatch`
deploys with explicit branch + slot inputs still work.
Each slot is a fully separate instance: its own port (`8903 + hash(slot)`),
`sakiot_preview_<slot>` database, `/var/lib/sakiot-preview-<slot>` +
`/srv/sakiot-preview-<slot>`, the `sakiot-preview-<slot>-web` unit, and
`<slot>.preview.patrykstyla.com`. All slots share one `/etc/sakiot/preview.env`;
the engine derives the per-slot values from the slot name at config load
(`Target::Preview` + slot arg). Preview slots deploy the **web server and
frontend only** — the `Bot` component is filtered out, so no FBI Agent and no
per-slot Discord bot; login is dev-login only (OAuth placeholders satisfy the
web server's startup config). Slots are bootstrapped and torn down with
`ops/preview-slot.sh` (Cloudflare DNS via API, or a single wildcard record).
Full docs in `PREVIEW.md`; remember to re-run `ops/update-deploy-engine.sh`
after changing `ops/` so the `preview-ci`/`preview-up`/`preview-remove` forced
command verbs and the sudo rule are installed.

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
4. The dispatched run skips duplicate CI, requires the exact tag/SHA promotion,
   re-verifies every digest, copies it into the production release directory,
   and deploys it. A missing or modified promotion fails before migrations or
   service changes. Manual/raw tag pushes still run full CI and can build on the
   VPS when no promotion exists.

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
/var/cache/sakiot/promotions
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

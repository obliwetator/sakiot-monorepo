# Production deployment

Production deploys run from GitHub-hosted Actions runners when a new `v*` tag
is pushed. The runner has read-only repository permission and sends only the
tag and commit SHA through a forced SSH command. Builds, tests, backups,
migrations, service changes, and health checks run on the VPS.

## VPS bootstrap

Install required tools: Git, Rust, Bun, `protoc`, OpenSSL development headers,
FFmpeg, `audiowaveform`, PostgreSQL client tools, SQLx CLI, `age`, `grpcurl`,
`jq`, `rsync`, Python 3, `flock`, and `sudo`.

As root:

```sh
ops/install-production.sh /root/github-deploy-key.pub
$EDITOR /etc/sakiot/production.env
systemctl enable sakiot-web.service
```

The installer creates the `sakiot` user, persistent directories, systemd
units, restricted `authorized_keys`, and a root-owned systemd command validator
behind narrowly scoped sudo rules. Validate
the generated key line contains `restrict` and the forced command. Keep manual
debug services under the developer account; production units are named
`sakiot-web.service` and `sakiot-fbi-agent@<release>.service`.

For the first release on a host still running the prior `tulipan` user units,
set `SAKIOT_LEGACY_BOT_UNIT`, `SAKIOT_LEGACY_BOT_GRPC`, and
`SAKIOT_LEGACY_WEB_ENABLED=1`. The deployer drains the old bot, stops the old
web service only after builds pass, and restores the old web service if new
health checks fail. These settings are ignored after production state exists.

Copy this repository's `ops` directory to `/usr/local/lib/sakiot-deploy` after
reviewing deployment-framework changes. Application release tags cannot modify
the root-owned SSH bootstrap by themselves.

## GitHub environment

Create a `production` environment without required reviewers and add:

- `DEPLOY_HOST`
- `DEPLOY_USER` (`sakiot`)
- `DEPLOY_SSH_KEY`
- `DEPLOY_KNOWN_HOSTS` (pre-verified host key, not live `ssh-keyscan` output)

Repository owner account should use a passkey or 2FA. Do not add deployment
secrets to pull-request workflows or use `pull_request_target`.

## Release

Push a new immutable release tag:

```sh
git tag -a v1.2.3 -m "v1.2.3"
git push origin v1.2.3
```

The deployer rejects invalid, moved, or previously successful tags. It locks
deployment state, verifies the tag commit, selects changed components, completes
all builds/tests before mutations, runs encrypted backup before migrations,
deploys bot/web/frontend, and records a manifest only after health checks pass.
Documentation-only tags are recorded as successful no-ops.

## Rollback

Run the `Roll back production` workflow with a prior tag. Rollback rebuilds all
application components and never reverses migrations or restores a database.
It blocks when migration files differ from current production unless
`allow_schema_mismatch` is explicitly selected after compatibility review.

## Runtime state

```text
/etc/sakiot/production.env
/var/lib/sakiot/data
/var/lib/sakiot/deploy
/srv/sakiot/releases
/srv/sakiot/current
/var/cache/sakiot
```

Release manifests are under `/srv/sakiot/releases/<release>/manifest.json`;
`/var/lib/sakiot/deploy/current.manifest` points to the last successful one.
Stopped releases are intentionally retained. Never remove a release directory
while its `sakiot-fbi-agent@...` unit is active or draining.

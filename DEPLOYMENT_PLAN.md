# Public Repository VPS Deployment Plan

## Summary

- Develop full stack in Windows WSL2.
- Keep repository public.
- Automatically deploy when a new `v*` tag is pushed.
- Run orchestration on a GitHub-hosted runner.
- Connect to VPS through restricted SSH.
- Perform checkout, compilation, migration, and deployment on VPS.
- Do not expose a webhook service or run a persistent GitHub runner on production VPS.

## Deployment Flow

1. GitHub Actions workflow triggers for newly pushed tags matching `v*`.
2. GitHub-hosted job receives only `contents: read`.
3. Job sends tag and commit SHA to VPS through restricted SSH.
4. VPS deploy wrapper:
   - Acquires exclusive deployment lock.
   - Validates tag and commit SHA format.
   - Fetches public repository over HTTPS.
   - Confirms tag resolves to supplied commit.
   - Rejects moved or reused release tags.
   - Compares commit with last successful deployment.
   - Builds, tests, and deploys affected components.
5. Deployment records tag and SHA only after all health checks pass.

Tag deletion must not trigger deployment.

## Component Selection

Use one repository-wide release tag, but deploy only changed components and
their dependents:

- `FBI-agent/**`: deploy bot.
- `web_server/**`: deploy web server.
- `sakiot_stage/**`: deploy frontend.
- `sakiot-paths/**`: deploy bot and web server.
- `sakiot-proto/**`: deploy bot and web server.
- Root `Cargo.toml`, `Cargo.lock`, or `.sqlx/**`: deploy bot and web server.
- `sakiot-db/migrations/**`: run database phase and deploy affected services.
- Deployment or shared operational files: deploy all relevant components.
- Documentation-only changes: successful no-op.
- First deployment or unknown dependency mapping: deploy everything.

## VPS Runtime Layout

Run production under dedicated `sakiot` user. Separate disposable source from
persistent runtime state:

```text
/etc/sakiot/production.env
/var/lib/sakiot/data
/var/lib/sakiot/deploy
/srv/sakiot/releases
/var/cache/sakiot
```

Production source checkout may be replaced or cleaned without touching secrets,
recordings, deployment state, or prior releases.

Keep current manual debug services under existing user and outside automated
production deployment.

## Restricted SSH

GitHub environment stores dedicated deployment private key. VPS public key entry
must enforce one forced deploy command and disable:

- Interactive shell
- PTY allocation
- Port forwarding
- Agent forwarding
- X11 forwarding

Forced command accepts only expected tag and SHA values. It must not evaluate
arbitrary shell input supplied by workflow.

## Build And Deployment

All affected builds and tests complete before database or service changes.

### Database

- Check for pending migrations.
- Run existing `sakiot-db/ops/backup/pre-migrate-backup.sh`.
- Abort deployment if encrypted backup or migration fails.
- Keep existing hourly/nightly backups and periodic restore tests.
- Never reverse production migrations automatically.

### FBI Agent

- Preserve current overlapping, drain-aware release behavior.
- Make `grpcurl` mandatory for production deployment.
- Add `Admin/CancelDrain` to recover old active instance when new release fails
  readiness.
- Never prune active draining releases.
- Retain several stopped prior releases for rollback.

### Web Server

- Build immutable release directory.
- Add unauthenticated `GET /healthz` that checks process and database readiness
  and returns release ID.
- Atomically switch current release symlink.
- Restart systemd service and verify health.
- Restore previous symlink and service when readiness fails.

### Frontend

- Build on VPS.
- Publish hashed assets first.
- Publish `index.html` and `version.json` last.
- Include release tag and commit SHA in `version.json`.
- Preserve older hashed assets for open browser sessions and cached HTML.

## GitHub Actions Security

- Use GitHub-hosted runners only.
- Set workflow permissions to `contents: read`.
- Pin third-party actions to immutable commit SHAs.
- Deployment secrets exist only in production environment.
- PR workflows receive no deployment secrets.
- Never use `pull_request_target` to execute contributor-controlled code.
- Configure workflow concurrency so production deployments never overlap.
- Do not cancel a deployment already modifying production.
- Protect repository owner account with passkey or 2FA.

Automatic tag deployment has no approval gate. Any valid new owner-created
`v*` tag starts production deployment immediately.

## Rollback

Provide manual workflow selecting a prior release tag.

- Rebuild and redeploy affected application components from selected tag.
- Do not automatically restore database backups or reverse migrations.
- Block rollback across newer migrations unless explicit schema-compatibility
  override is supplied.
- Keep deployment manifest containing tag, SHA, changed components, migration
  state, release paths, and timestamps.

## Local Development

Run application tools natively inside WSL2:

- Rust toolchain
- Bun
- `protoc`
- OpenSSL development libraries
- FFmpeg and `ffprobe`
- `audiowaveform`
- PostgreSQL client
- Docker Desktop WSL integration

Provide development Docker Compose configuration containing isolated PostgreSQL,
for example on host port `54320`. Run bot debug, web server, and Vite directly
inside WSL2 using local `.env` and local data directory.

Local secrets and runtime data remain untracked. Committed frontend development
environment files may contain public `VITE_*` values only.

## Container Transition

Expose one stable deployment interface:

```text
ops/deploy release <tag> <sha>
```

Initial implementation builds native binaries and controls systemd services.
Future implementation replaces internals with VPS-side Docker Compose builds
and deployments without changing GitHub workflow.

First container phase includes:

- FBI Agent
- Web server

Initially retain:

- Host Nginx
- Static frontend deployment
- Host PostgreSQL
- Existing persistent paths
- Existing release IDs and health contracts

## Verification

- Unit-test path-to-component mapping.
- Test first-release and unknown-change fallback.
- Test invalid, deleted, reused, and moved tags.
- Test forced-command SSH restrictions.
- Test deployment locking.
- Test migration backup failure blocks deployment.
- Test failed bot readiness cancels old-instance drain.
- Test failed web health restores previous release.
- Test frontend publication order.
- Smoke-test full WSL2 development stack.
- Smoke-test bot-only, web-only, frontend-only, shared-crate, and migration
  releases.

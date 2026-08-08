# Preview instances (branch deploys)

`<slot>.preview.patrykstyla.com` hosts web server + frontend instances on the
same VPS as production and main staging. Each **slot** is a fully separate
instance — own DB, ports, units, subdomain — that any branch can be deployed
to, so several branches can be tested side by side without touching the
staging instance that always tracks `main`.

**Previews run no Discord bot** (only `web_server` + the frontend) and use
**dev login only** — no Discord OAuth application is involved anywhere. Bot
behavior is exercised on staging; the bot's per-token gateway limit is what
made per-slot bots painful, and dropping them makes slots cheap.

## Automatic per-branch lifecycle (recommended)

Every push to a non-`main` branch (docs-only pushes skipped) is deployed
automatically: the `Deploy preview` workflow derives a **slot from the branch
name**, provisions it on the VPS if missing, deploys the pushed commit after
CI passes, and **tears the slot down when the branch is deleted** (e.g. after
merging a PR with auto-delete). Nothing per branch is needed:

```sh
# One-time setup, as root on the VPS:
ops/update-deploy-engine.sh
```

The workflow SSHes through the restricted forced command:

- `preview-up <slot>` — runs `ops/preview-slot.sh <slot>` as root (idempotent)
- `preview-ci <slot> <sha>` — deploys the commit (existing verb)
- `preview-remove <slot>` — runs `ops/preview-slot.sh <slot> --remove`

Branch → slot mapping is deterministic and reversible: lowercase, every
non-`[a-z0-9]` run becomes `-`, trimmed and truncated to 32 chars
(`feature/Clip-Editor v2` → `feature-clip-editor-v2`). `main` is excluded —
the staging instance already tracks it. Two branches can collide on one slot
(e.g. `feature/foo` and `feature_foo`); the later push wins the slot.

The restricted `sakiot` deploy key can now run `preview-slot.sh` as root via
a narrow sudo rule (`ops/sudoers/sakiot-deploy`); the script is root-owned,
read-only for `sakiot`, and validates its slot argument, so it can only
create/remove DNS records, vhosts, units, certs, and databases for valid slot
names.

Manual deploys still work: **Actions → Deploy preview → Run workflow**, pick
the `branch` and the `slot`. The slot is provisioned on demand too, so the
manual bootstrap below is optional.

## Slot lifecycle (one time per slot, as root)

Manual bootstrap — only needed when you want to skip the automatic flow (or
for slots not tied to a branch):

`ops/preview-slot.sh` automates the slot: Cloudflare DNS record, database,
systemd unit, nginx vhost, and (optionally) the HTTPS cert. The env file is
**shared** — `/etc/sakiot/preview.env` is created once and reused by every
slot; the deploy engine derives each slot's port, database, dirs, units, and
subdomain from the slot name.

```sh
# 1. Refresh the deploy framework so the preview-ci verb exists:
ops/update-deploy-engine.sh

# 2. Create a slot. The Cloudflare token and certbot email come from the
#    shared env file, so nothing needs to be passed:
ops/preview-slot.sh clip-editor
#    (repeat for more slots: ops/preview-slot.sh other-branch)

# 3. Set the shared credentials once in /etc/sakiot/preview.env:
#    CLOUDFLARE_API_TOKEN (DNS records), DEV_ACCOUNT_ID + DEV_LOGIN_SECRET
#    (the only login is dev login), CERTBOT_EMAIL (slot HTTPS certs), and
#    JWT/registry/DB secrets. DISCORD_CLIENT_ID / DISCORD_CLIENT_SECRET are
#    startup placeholders — any non-empty value works since OAuth is never
#    used on preview hosts. API host variables (VITE_API_URL, COOKIE_DOMAIN,
#    CORS/opener origins) are derived per slot automatically — nothing to
#    edit for those.

# 4. Deploy: Actions -> Deploy preview -> slot=<slot>, branch=<branch>.

# Teardown when the branch is done (env file and other slots are untouched):
ops/preview-slot.sh clip-editor --remove
```

`ops/preview-slot.sh` details:

- The web port is **deterministic**: `8903 + hash(slot)` (same function as the
  engine's `slot_port`), so the vhost, unit, and deployed service agree.
- Creates/updates the `A` record `<slot>.preview.patrykstyla.com` via the
  Cloudflare API; `--no-dns` skips this when a wildcard record is in place.
- Creates `/etc/sakiot/preview.env` from `ops/preview.env.example` only if it
  does not exist yet (base values; the engine namespaces them per slot).
- Creates the per-slot directories owned by `sakiot` (data/deploy/backups
  under `/var/lib/sakiot-preview-<slot>`, releases/current under
  `/srv/sakiot-preview-<slot>`, the per-slot cache, and the frontend web
  root) — the deploy engine expects these to pre-exist.
- Installs the web systemd unit from the staging template (no bot unit —
  previews run no FBI Agent) and the nginx vhost from
  `ops/nginx/preview-slot.conf.example`; runs `certbot --nginx` when
  `CERTBOT_EMAIL` is set.
- `--remove` reverses everything for that slot: DNS record, cert, vhost,
  unit, database — the shared env file stays.

### Engine-side slot derivation

A `preview-ci <slot> <sha>` deploy loads the shared `preview.env` and rewrites
these tokens per slot (mirroring the old per-slot files):

| token in preview.env        | per-slot value                  |
|-----------------------------|---------------------------------|
| `PORT` (8903 base)          | `8903 + hash(slot)`             |
| `sakiot_preview`            | `sakiot_preview_<slot>`         |
| `sakiot-preview` (paths/units) | `sakiot-preview-<slot>`      |
| `preview.patrykstyla.com`   | `<slot>.preview.patrykstyla.com` |

The per-slot port is also written into each release's `web/service.env`, so
every slot's web server binds its own port while reading the shared env file.
The same file carries the slot's `COOKIE_DOMAIN`, `CORS_ALLOWED_ORIGIN`,
`OAUTH_ALLOWED_OPENER_ORIGINS`, and `DISCORD_REDIRECT_URI` (later
`EnvironmentFile` wins in the systemd unit), and the frontend build's
`VITE_API_URL` is rewritten before `bun build` runs.

## DNS and TLS — no wildcard needed

Two equivalent setups, pick one:

- **Wildcard record (recommended, zero per-slot DNS):** create ONE record
  `*.preview.patrykstyla.com -> <VPS IP>` (Cloudflare console or API, once).
  Every slot then resolves automatically and `ops/preview-slot.sh --no-dns`
  skips DNS work. Certbot still issues per-slot certificates over HTTP-01
  (port 80 must be reachable), which works because each subdomain resolves
  through the wildcard.
- **Per-slot records:** the slot script creates and later deletes each slot's
  `A` record through the Cloudflare API — nothing to do per branch. With a
  Cloudflare API token you can also switch to a wildcard cert via DNS-01
  (`certbot-dns-cloudflare`) later; not required.

## Layout (one slot)

| thing            | staging                          | preview slot                     |
|------------------|----------------------------------|----------------------------------|
| web port         | 8901                             | **8903+ (auto-assigned)**        |
| database         | `sakiot_staging`                 | `sakiot_preview_<slot>`          |
| Discord bot      | DEBUG bot                        | **none (web + frontend only)**   |
| web unit         | `sakiot-staging-web.service`     | `sakiot-preview-<slot>-web.service` |
| data dir         | `/var/lib/sakiot-staging`        | `/var/lib/sakiot-preview-<slot>` |
| releases         | `/srv/sakiot-staging`            | `/srv/sakiot-preview-<slot>`     |
| cache            | `/var/cache/sakiot-staging`      | `/var/cache/sakiot-preview-<slot>` |
| env file         | `/etc/sakiot/staging.env`        | **`/etc/sakiot/preview.env` (shared)** |
| frontend domain  | staging.patrykstyla.com          | **<slot>.preview.patrykstyla.com** |

Cargo target dir, promotions, and the SQLx test master are shared with the
other instances, as documented in `ops/README.md`.

## Notes

- The workflow reuses the `staging` environment's `DEPLOY_*` secrets; the
  forced command chooses the instance from the slot argument.
- Deploys to different slots run in parallel (per-slot concurrency groups).
- Dev login works on preview hosts (`sakiot-stage` shows the button on hosts
  containing `preview`), gated by `DEV_ACCOUNT_ID`/`DEV_LOGIN_SECRET` in the
  slot's env file.
- Media archive is disabled by default in the example env; flip
  `SAKIOT_MEDIA_ARCHIVE_ENABLED` when a branch needs it.
- Check a deployed slot with `sudo -u sakiot sakiot-deploy status preview <slot>`
  (local, VPS).

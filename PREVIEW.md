# Preview instances (branch deploys)

`<slot>.preview.patrykstyla.com` hosts web server + frontend instances on the
same VPS as production and main staging. Each **slot** is a fully separate
instance — own DB, ports, units, subdomain — that any branch can be deployed
to, so several branches can be tested side by side without touching the
staging instance that always tracks `main`.

**Previews run no Discord bot** (only `web_server` + the frontend). Bot
behavior is exercised on staging; the bot's per-token gateway limit is what
made per-slot bots painful, and dropping them makes slots cheap. Web login
still works: OAuth reuses the staging application's client id/secret with a
per-slot redirect URI — no new Discord application anywhere.

Deploys are manual: **Actions → Deploy preview → Run workflow**, pick the
`branch` and the `slot` to deploy it into. The workflow runs the standard CI,
then SSHes `preview-ci <slot> <sha>` through the restricted forced command.
Re-deploying a new commit of the same branch, or a different branch into the
same slot, overwrites that slot only; other slots and main staging keep
running whatever they had.

## Slot lifecycle (one time per slot, as root)

`ops/preview-slot.sh` automates the whole slot: Cloudflare DNS record, env
file, database, systemd units, nginx vhost, and (optionally) the HTTPS cert.

```sh
# 1. Refresh the deploy framework so the preview-ci verb exists:
ops/update-deploy-engine.sh

# 2. Create a slot. Needs a Cloudflare API token (Zone:DNS edit) in
#    CLOUDFLARE_API_TOKEN; certbot email in CERTBOT_EMAIL for HTTPS.
CLOUDFLARE_API_TOKEN=... CERTBOT_EMAIL=you@example.com ops/preview-slot.sh clip-editor

# 3. No Discord bot needed. Just set the shared OAuth credentials in
#    /etc/sakiot/preview-clip-editor.env: copy DISCORD_CLIENT_ID and
#    DISCORD_CLIENT_SECRET from the staging application, and add
#    https://clip-editor.preview.patrykstyla.com/api/discord_login to that
#    application's OAuth redirect URIs (Dev Portal -> OAuth2 -> Redirects).

# 4. Deploy: Actions -> Deploy preview -> slot=clip-editor, branch=<branch>.

# Teardown when the branch is done:
CLOUDFLARE_API_TOKEN=... ops/preview-slot.sh clip-editor --remove
```

`ops/preview-slot.sh` details:

- Picks the next free web port (8903, 8904, ...) from existing
  `/etc/sakiot/preview-*.env` files, or takes `--port N`.
- Creates/updates the `A` record `<slot>.preview.patrykstyla.com` via the
  Cloudflare API; `--no-dns` skips this when a wildcard record is in place.
- Renders `/etc/sakiot/preview-<slot>.env` from `ops/preview.env.example`
  (paths, ports, DB name, domain, units all namespaced to the slot) and the
  web systemd unit from the staging template (no bot unit — previews run no
  FBI Agent).
- Installs the nginx vhost from `ops/nginx/preview-slot.conf.example` and
  runs `certbot --nginx` when `CERTBOT_EMAIL` is set.
- `--remove` reverses everything: DNS record, cert, vhost, unit, DB, env file.

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
| env file         | `/etc/sakiot/staging.env`        | `/etc/sakiot/preview-<slot>.env` |
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

# Local fixtures and shell completion

Detailed reference for `cargo dev` shell integration and production-shaped
fixture workflows.

## Shell completion

To enable shell completion for `cargo dev`, generate the completion script once
for your shell. The generated script is static and only needs to be regenerated
when the CLI commands or flags change. For zsh:

```sh
mkdir -p ~/.zfunc
rustup completions zsh cargo > ~/.zfunc/_cargo
cargo dev completions zsh > ~/.zfunc/_cargo-dev
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

The first generated file provides zsh's Cargo dispatcher; it is required for
Cargo to delegate `cargo dev` completion to the `_cargo-dev` script. If your
shell already loads Cargo's `_cargo` completion, only the second command is
needed.

For bash, save it where bash-completion discovers Cargo subcommands:

```sh
mkdir -p ~/.local/share/bash-completion/completions
cargo dev completions bash > ~/.local/share/bash-completion/completions/cargo-dev
```

Fish and PowerShell scripts are also available with `completions fish` and
`completions powershell`. Completion covers commands, flags, and fixed enum
values; fixture URLs, UUIDs, and remote numeric IDs remain free-form.

## Production-shaped fixtures

The synthetic seed leaves the recordings list empty. To test the recordings
UI with real audio, waveforms, and metadata, pull a sample from a deployed
instance over your personal SSH access (read-only on the VPS side; nothing
is committed to the repository). Set `SAKIOT_DEV_SSH` in the git-ignored `.env`
to avoid entering the SSH target when the startup prompt requests recordings:

```sh
SAKIOT_DEV_SSH=user@vps-host
# Or invoke the standalone command directly. It reports the server counts for
# the live-test guild and asks how many newest recordings to download:
SAKIOT_DEV_SSH=user@vps-host cargo dev fixtures sync
# Explicit counts are non-interactive and only select categories you name:
SAKIOT_DEV_SSH=user@vps-host cargo dev fixtures sync --recordings 20
SAKIOT_DEV_SSH=user@vps-host cargo dev fixtures sync --recordings 20 --clips 5 --stamps 5
# Every eligible recording, clip, and stamp from the last seven days:
SAKIOT_DEV_SSH=user@vps-host cargo dev fixtures sync --days 7
# The default guild is 362257054829641758; override it only when intentional:
SAKIOT_DEV_SSH=user@vps-host cargo dev fixtures sync --guild <id> --recordings 20
```

The interactive sync prints how many finalized recordings, clips, and stamps the
selected guild has on the server, then asks for a newest-first recording count
(10 is the default). It does not offer an implicit “all” choice. Use
`--recordings all` explicitly when you really want every recording. Clips and
stamps are only copied when their flags are supplied. `--days N` is the global
time-window form: it copies every eligible recording, clip, and stamp whose
source timestamp falls within the last N days:

```
[dev] staging guild 362257054829641758 has 509 finalized recording(s), 36 clip(s), and 123 stamp(s) on the server
[dev] download latest recording(s) (server has 509; enter a number or none) [10]:
```

Bulk sync is scoped to guild `362257054829641758` by default, including startup
fixture sync. A different guild requires an explicit `--guild` or
`SAKIOT_DEV_FIXTURE_GUILD`; no command silently samples unrelated guilds.

```sh
cargo dev fixtures sync --recordings all             # every recording in the live-test guild
cargo dev fixtures sync --recordings 20 --clips 5    # newest recordings + five clips, same guild
cargo dev fixtures sync --days 7                     # every category from last 7 days
cargo dev fixtures sync --guild 362257054829641758 --recordings 20
```

`--days` is mutually exclusive with the individual category count flags. For
the global recent window, use `--days N` by itself.

A positive `--recordings` count replaces the previously managed fixture
recordings and media; it does not touch unrelated local data. It is the only
selection that re-answers "which recordings do I want?", so it is the only one
that replaces anything (and the same applies to the `all` default):
`--recordings none` keeps the
existing set even when the same run asks for clips or stamps, and
`--clips`/`--stamps` only ever add. Tracking rows in
`media_objects` for a replaced recording are dropped with it — the FK is
`ON DELETE RESTRICT`, and the local server re-reconciles what it still has on
its next pass. The remote export and media download complete before replacement
begins, so a remote failure leaves the previous set intact.

### Pulling one specific recording, clip or stamp

When a particular production file misbehaves or makes a good test sample, hand
its dashboard URL straight to `fetch`:

```sh
cargo dev fixtures fetch https://patrykstyla.com/dashboard/362257054829641758/audio/763782256980131892/2026/7/1785336946781-161172393719496704
cargo dev fixtures fetch https://patrykstyla.com/dashboard/362257054829641758/audio/session/384
cargo dev fixtures fetch https://patrykstyla.com/dashboard/362257054829641758/clips/2ae61e36-ada8-45df-baed-8aa26386c826
cargo dev fixtures fetch 1785336946781-161172393719496704   # bare file name
cargo dev fixtures fetch 2ae61e36-ada8-45df-baed-8aa26386c826  # bare clip UUID
cargo dev fixtures fetch 384                                # bare id, resolved remotely
cargo dev fixtures fetch 1204 --kind stamp                   # stamps have no URL of their own
```

A file URL pulls that fragment; a session URL pulls every fragment of the
logical session. Both also pull the parent `recording_sessions` row plus its
gaps and timeline events. Unlike a recording sync, this **adds** to the fixture set
instead of replacing it, and re-running the same fetch is a no-op.

### Clips and stamps travel with their session

A clip or a stamp is an annotation on a recording session, so neither means
anything on its own and the selection is widened in both directions:

- naming a clip or a stamp pulls the session behind it **whole** — every
  fragment, not just the one the annotation happens to sit on. Both listings are
  gated on the viewer being allowed to see *every* fragment of the session, so a
  partial import would land rows nothing can display;
- naming a recording or a session pulls the clips and stamps hanging off it, so
  an imported session looks the way it does in the source instead of arriving
  stripped of every annotation.

A pre-session clip names its source recording in `original_file_name` rather than
through `recording_session_id`, and that recording is pulled too. A session whose
fragments have all been reaped still comes across when a clip or stamp points at
it — it is the only thing the annotation hangs off.

Clip media (`clips/{YYYY}/{MM}/{uuid}.ogg`) is downloaded alongside the
recordings, and archive-pruned clips are materialized first, the same way
recordings are.

`clips.clip_id` is a UUID minted at capture time, so it survives the import
untouched: a `/clips/<uuid>` URL works verbatim against the destination, exactly
as a file URL does. Stamps have no stable key — `stamps.id` is a dense sequence
that means something different in every environment — so their ids are always
reissued, and a re-import recognizes a stamp it has already seen by what actually
identifies an occurrence: who stamped whom, in which channel, at which
`stamp_ts`. The stamp page is per guild (`/stamps/<guild_id>`), which is what the
run prints.

The audio event timeline draws from three tables, so all three come across.
`recording_session_events` feeds the "recording" lane; `voice_state_events` feeds
mute, deafen, suppress, channel and media; `voice_connection_events` feeds
"connection". The latter two are keyed by guild and wall-clock time rather than
by recording, so they are scoped to the span the imported fragments cover, plus
a margin (`SAKIOT_DEV_EVENT_MARGIN`, default 5 minutes) — the connect operation
that starts a session completes just before its first fragment and would
otherwise be missed. Events for every user in the channel are pulled, not just
the recorded one, along with their names.

The host in the URL picks the source, so a `patrykstyla.com` link reads from
production and a `staging.patrykstyla.com` one from staging. Override with
`--source production|staging`, or set `SAKIOT_DEV_SOURCE` in `.env` for bare ids. A
bare number can be either an `audio_files.id` or a `recording_sessions.id`; the
CLI looks it up and requires `--kind recording` or `--kind session` if both match.
`stamps.id` is deliberately left out of that guess — it is a third dense sequence,
so including it would make almost every bare number ambiguous; select a stamp
with `--kind stamp`. A bare UUID can only be a clip, so it needs no
disambiguation.

URLs stay portable. `file_name`, guild, channel, year and month survive the
import unchanged, and a session keeps its source `recording_sessions.id` when the
destination has that id free — so both URL shapes work verbatim, only the host
changes. If the destination already uses that id the session gets a fresh one
rather than displacing anything, and the run says so:

```
[dev] session 376 was already taken there; imported as 41
```

`audio_files.id` is always reissued: nothing addresses a recording by it, while
`media_objects` and `stamps` reference it, so preserving it would only risk
collisions. An imported stamp is repointed at the id its recording landed on, so
it stays attached to the fragment it was placed on rather than to whichever row
happens to hold that number in the destination. The command prints the URLs to
open when it finishes.

### Copying production data into staging

`--destination staging` puts the same selection into the staging instance instead of
the local dev database, for reproducing something on `staging.patrykstyla.com`:

```sh
cargo dev fixtures fetch https://patrykstyla.com/dashboard/362.../audio/session/384 --destination staging
cargo dev fixtures fetch https://patrykstyla.com/dashboard/362.../clips/2ae61e36-... --destination staging
cargo dev fixtures fetch 1204 --kind stamp --source production --destination staging
```

It takes one named recording, session, clip or stamp — never a bulk or random
sample, which is not what reproducing a specific report needs.

It asks for confirmation first, and the copy is deliberately obvious afterwards:

- every imported session gets an `imported_from_production` event at offset 0 of
  its staging timeline, carrying the origin URL, the origin session id, who ran
  the import, and when — visible as a marker in the session player;
- each recording, clip and stamp is appended to
  `/var/lib/sakiot-staging/data/.imported-from-production.list` — a stamp by its
  production id and `stamp_ts`, since staging reissues the id;
- the run prints a banner naming the target database, the media directory, and
  how many recordings, clips and stamps are about to cross.

Importing the rows and files is not enough to make a recording *reachable*. The
listing goes through `visible_channels_for_user`, which returns 403 unless the
viewing account has a `user_guilds` row for that guild, and staging is reached
through `dev_login` — a token for `DEV_ACCOUNT_ID`, never a Discord identity, so
nothing populates that table on its own. The import therefore reads
`DEV_ACCOUNT_ID` out of `staging.env` and grants that account access to the
imported guild, listed as `Imported from production <id>` in the guild picker.
Override the account with `SAKIOT_DEV_STAGING_ACCOUNT_ID` if the env file is not
readable.

`guilds_present` is topped up for the same reason. The staging bot rewrites that
table on startup and is not a member of a production guild, so the entry
disappears on its next restart — re-run the fetch to restore it.

The command finishes with a pass/fail line per precondition — media files on
disk, the imported clip and stamp rows, the `@everyone` role, voice channel rows,
`guilds_present`, and the `dev_login` account's access — instead of reporting
success on data that nothing can serve.

### Remote access details

On the VPS itself the command detects the source data directory and switches to
local mode automatically — no `SAKIOT_DEV_SSH` needed. By default, the export
runs on the VPS using the source's env file (`/etc/sakiot/production.env` or
`/etc/sakiot/staging.env`) when readable, then tries `sudo -n -u postgres psql`,
then direct `psql`. For a nonstandard database setup, override the remote
command:

```sh
SAKIOT_DEV_REMOTE_PSQL="sudo -n -u postgres psql" cargo dev fixtures sync
```

Each candidate is accepted only if it actually holds `SELECT` on `audio_files`,
`recording_sessions`, `clips` and `stamps`, so a role that can connect but was
never granted the tables is rejected up front instead of failing mid-export.
Writing into staging is probed the same way, for `INSERT`.

Archive-pruned recordings and clips are materialized and SHA-256 verified before
`rsync` (`web_server media restore --audio-file-ids` / `--clip-ids`). The workflow
first tries the source environment/binary directly, then `sudo -n -u sakiot`.

Recordings and clips are checked on the source disk first, so the archive is only
asked for genuinely absent media. If a recording is still unavailable, sync warns
and imports the metadata without that physical file; a missing recording is not a
mission-critical sync failure. Clips behave the same way and are reported as
metadata without audio. That also keeps the workflow useful against a deployment
older than `media restore --clip-ids`, which would otherwise reject the call
outright.

Override unusual layouts with
`SAKIOT_DEV_REMOTE_WEB_BINARY`, `SAKIOT_DEV_REMOTE_ENV_FILE`,
`SAKIOT_DEV_REMOTE_DB`, `SAKIOT_DEV_REMOTE_DATA`, or a complete
`SAKIOT_DEV_REMOTE_HYDRATE` command — which is invoked as
`$SAKIOT_DEV_REMOTE_HYDRATE --audio-file-ids <ids>` or
`--clip-ids <ids>`, so it must accept the selector flag. Writing into staging uses
`SAKIOT_DEV_STAGING_ENV_FILE`, `SAKIOT_DEV_STAGING_DB`,
`SAKIOT_DEV_STAGING_DATA`, `SAKIOT_DEV_STAGING_PSQL` and
`SAKIOT_DEV_STAGING_RSYNC_PATH`.

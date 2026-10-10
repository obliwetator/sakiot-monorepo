# fbi-agent

The Discord bot: the only component that talks to Discord. It joins voice
channels and records them, handles slash commands, keeps the guild cache and
member/presence projections in PostgreSQL, and serves the `Jammer` and `Admin`
gRPC services. How it fits with the other components is in
[architecture](../docs/architecture.md).

Recording is on by default. A member stops it for themselves in one server
with `/recording opt-out` (or the web app's account menu) and resumes with
`/recording opt-in`; open recordings close within a second and earlier ones are
kept.

## Configuration

The build profile picks the Discord application (`src/config.rs`): a debug
build reads `DISCORD_TOKEN_DEBUG` and `APPLICATION_ID_DEBUG`, a release build
the `*_RELEASE` pair. That is why staging, a release build, puts the DEBUG
bot's credentials in the `*_RELEASE` variables. The default gRPC address
follows the profile too (`[::1]:50053` debug, `[::1]:50052` release);
`GRPC_ADDR` overrides it. A release build refuses to start without
`FBI_AGENT_REGISTRY_SECRET`.

## Releases and draining

Each release runs as its own unit, `sakiot-fbi-agent@<release_id>.service`
(`sakiot-staging-fbi-agent@...` on staging), from its own directory:

```text
/srv/sakiot/releases/<release_id>/fbi-agent/fbi_agent
/srv/sakiot/releases/<release_id>/fbi-agent/service.env
```

The deploy engine writes `service.env` with `BOT_ROLE`, `BOT_INSTANCE_ID`,
`RELEASE_ID`, a unique loopback `GRPC_ADDR`, `SAKIOT_DATA_DIR`, and
`DRAIN_TIMEOUT_SECONDS=0`, which makes a draining instance wait as long as its
voice connections last.

A deploy starts the new release, then drains the old one over the `Admin`
service; the old instance exits once it has no voice connections. The units
use `Restart=on-failure`, so a cleanly drained release stays stopped, and
`TimeoutStopSec=infinity`, so systemd never kills an instance that is still
recording. Only the newest active release is enabled on boot.

During a handover:

- Only the active release writes the guild cache (guilds, roles, channels,
  overwrites, member roles, `guilds_present`). A draining release keeps
  recording but skips those writes and the periodic resync. A drain is
  cancelled only when a deploy rolls back; the instance then resumes writing
  and resyncs at once.
- The member and voice presence projections follow ownership instead: an
  instance keeps writing the guilds it claimed in `guild_projection_state`,
  even while draining, until the new release claims them from a complete
  snapshot. A draining instance never claims, and a cancelled drain refreshes
  and reclaims every guild (`src/projections.rs`).

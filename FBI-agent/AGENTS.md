# FBI Agent Deployment Notes

## Deployment Rule

Do not deploy, restart, stop, enable, disable, or kill any `fbi-agent` systemd service unless the user explicitly asks for deployment or service control.

For verification during normal code work, only run Cargo commands such as:

- `cargo check`
- `cargo test`
- `cargo build`
- `cargo build --release`

Do not use `deploy/*.sh`, `systemctl --user ...`, `grpcurl ...`, or service-management commands unless manually instructed.

## Current Runtime Model

Production bot releases run as system services:

```text
sakiot-fbi-agent@<release_id>.service
```

The root deployment framework and systemd template live under `../ops/`.

Each release has its own immutable-ish binary and env file:

```text
releases/<release_id>/fbi_agent
releases/<release_id>/service.env
```

`service.env` contains:

```env
BOT_ROLE=active
BOT_INSTANCE_ID=<host>-<release_id>
GRPC_ADDR=<unique localhost port>
DRAIN_TIMEOUT_SECONDS=0
```

`DRAIN_TIMEOUT_SECONDS=0` means a draining instance waits forever until voice is empty.

## Deploy Flow

Production deploys are triggered by strict `vX.Y.Z` tags through GitHub Actions.
Use `../ops/release`; deployment implementation and rollback live under
`../ops/`. See `../ops/README.md`.

## Current Important Service Settings

Release units use:

```ini
Restart=on-failure
TimeoutStopSec=infinity
```

Reason:

- `Restart=on-failure` prevents a cleanly drained old release from restarting.
- `TimeoutStopSec=infinity` prevents systemd from killing a draining instance while it is still recording.

## Proto Location

The shared gRPC contract lives in the sibling `sakiot-proto` crate:

```text
../sakiot-proto/proto/fbi_agent.proto
```

Both `FBI-agent` and `web_server` depend on `sakiot-proto` for generated Rust
types instead of using a repo-local proto symlink.

## Current Expected State

Normal post-deploy state can include:

- one active release handling Discord Gateway/events/commands
- zero or more old draining releases while they still have voice connections

Only the newest active release should be enabled on boot.

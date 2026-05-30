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

The bot runs as user-space systemd release instances:

```text
fbi-agent@<release_id>.service
```

The user-space template lives at:

```text
~/.config/systemd/user/fbi-agent@.service
```

The repo copy of that template lives at:

```text
deploy/systemd/user/fbi-agent@.service
```

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

Deploy script:

```bash
deploy/drain-deploy.sh [release_id]
```

If `release_id` is omitted, the script generates a UTC timestamp.

The script:

1. Builds with `cargo build --release`.
2. Creates `releases/<release_id>/`.
3. Copies `target/release/fbi_agent` to `releases/<release_id>/fbi_agent`.
4. Writes `releases/<release_id>/service.env` with a unique localhost gRPC port.
5. Installs/reloads the user systemd template.
6. Calls old active instance `Admin/StartDrain` over gRPC.
7. Starts and enables `fbi-agent@<release_id>.service`.
8. Writes the new gRPC address to `releases/.deploy/current-grpc-addr`.
9. Calls old active instance `Admin/ShutdownWhenEmpty`.
10. Disables older enabled `fbi-agent@*.service` releases.

The old release remains running only if it still owns voice connections. It exits after voice becomes empty.

## Drain And Force Stop

Status script:

```bash
deploy/drain-status.sh
```

It lists active `fbi-agent@*.service` instances and calls `Admin/GetDrainStatus` for each release gRPC address.

Force-stop script:

```bash
deploy/force-stop-drain.sh
```

It is interactive:

1. Lists active release units.
2. Shows selected unit drain status.
3. Requires typing `FORCE`.
4. Calls `Admin/ForceShutdown`.
5. If still running, requires typing `KILL` before sending SIGKILL.

Use force stop only when accepting interrupted recordings.

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

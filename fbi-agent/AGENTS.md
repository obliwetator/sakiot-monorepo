# fbi-agent agent rules

Never deploy, restart, stop, enable, disable or kill an `fbi-agent` systemd
service, and never run `../ops/deploy`, `../ops/release`, `systemctl`,
`grpcurl` or other service-management commands, unless the user explicitly
asks for deployment or service control. A running agent is recording people.

For verification, use Cargo only (`cargo check`, `cargo build`,
`cargo build --release`, `cargo test`). Database tests use `#[sqlx::test]` and
must never run against the runtime `DATABASE_URL`: load the root `.env`, start
PostgreSQL with `cargo dev db up`, and run them with
`DATABASE_URL="$SAKIOT_TEST_DATABASE_URL"`.

The release and drain model is described in [README.md](README.md).

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use tonic::transport::Channel;

use crate::proto::jammer::{JamData, jammer_client::JammerClient};

static FAILURE_COUNTER: OnceLock<Counter<u64>> = OnceLock::new();

/// Bound the eager TCP/HTTP2 connect; without it a firewalled agent blocks the
/// caller on the OS-level connect timeout (~2 minutes).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Bound the whole RPC. The agent may download the clip from the archive on
/// first play, so this must cover a large transfer, but a hung agent must not
/// pin an actix worker thread forever.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

fn failure_counter() -> &'static Counter<u64> {
    FAILURE_COUNTER.get_or_init(|| {
        opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
            .u64_counter("fbi_agent_grpc_failures")
            .with_description("Outbound gRPC failures from web_server to FBI agent")
            .build()
    })
}

pub fn record_failure(operation: &'static str) {
    failure_counter().add(1, &[KeyValue::new("operation", operation)]);
}

pub async fn connect_jammer(
    address: String,
) -> Result<(String, JammerClient<Channel>), tonic::transport::Error> {
    let channel = tonic::transport::Endpoint::from_shared(address.clone())?
        .connect_timeout(CONNECT_TIMEOUT)
        .connect()
        .await?;
    Ok((address, JammerClient::new(channel)))
}

/// Builds a `jam_it` request with the call timeout applied.
pub fn jam_request(data: JamData) -> tonic::Request<JamData> {
    let mut request = tonic::Request::new(data);
    request.set_timeout(CALL_TIMEOUT);
    request
}

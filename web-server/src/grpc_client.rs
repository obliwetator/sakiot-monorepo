use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use sakiot_proto::INTERNAL_SECRET_HEADER;
use tonic::metadata::errors::InvalidMetadataValue;
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

/// Builds a `jam_it` request with the call timeout applied, carrying the
/// agent's internal secret when one is configured. The agent rejects calls
/// without it (see fbi-agent's `grpc::auth`).
pub fn jam_request(
    data: JamData,
    secret: Option<&str>,
) -> Result<tonic::Request<JamData>, InvalidMetadataValue> {
    let mut request = tonic::Request::new(data);
    request.set_timeout(CALL_TIMEOUT);
    if let Some(secret) = secret {
        request
            .metadata_mut()
            .insert(INTERNAL_SECRET_HEADER, secret.parse()?);
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jam() -> JamData {
        JamData {
            clip_name: "clip".into(),
            guild_id: 1,
            user_id: 2,
        }
    }

    #[test]
    fn jam_requests_carry_the_configured_secret() {
        let request = jam_request(jam(), Some("s3cret")).unwrap();
        assert_eq!(
            request.metadata().get(INTERNAL_SECRET_HEADER).unwrap(),
            "s3cret"
        );

        let request = jam_request(jam(), None).unwrap();
        assert!(request.metadata().get(INTERNAL_SECRET_HEADER).is_none());
    }

    #[test]
    fn a_secret_that_cannot_be_metadata_is_an_error() {
        assert!(jam_request(jam(), Some("line\nbreak")).is_err());
    }
}

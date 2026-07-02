use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use tonic::transport::Channel;

use crate::proto::jammer::dashboard_client::DashboardClient;
use crate::proto::jammer::jammer_client::JammerClient;

static FAILURE_COUNTER: OnceLock<Counter<u64>> = OnceLock::new();

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

pub async fn connect_dashboard(
    address: String,
) -> Result<(String, DashboardClient<Channel>), tonic::transport::Error> {
    let client = DashboardClient::connect(address.clone()).await?;
    Ok((address, client))
}

pub async fn connect_jammer(
    address: String,
) -> Result<(String, JammerClient<Channel>), tonic::transport::Error> {
    let client = JammerClient::connect(address.clone()).await?;
    Ok((address, client))
}

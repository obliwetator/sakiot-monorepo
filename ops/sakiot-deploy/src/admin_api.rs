//! FBI Agent Admin gRPC client. Replaces the grpcurl invocations in
//! deploy-release.sh; timeouts mirror `grpcurl -max-time 3` with curl-style
//! 2s connect budget.

use std::time::Duration;

use anyhow::{Context, Result};
use sakiot_proto::fbi_agent::admin_client::AdminClient;
use sakiot_proto::fbi_agent::{DrainRequest, DrainStatus, Empty};
use tonic::transport::Endpoint;

pub trait AdminApi {
    fn start_drain(&self, address: &str, reason: &str) -> Result<()>;
    fn cancel_drain(&self, address: &str, reason: &str) -> Result<()>;
    fn shutdown_when_empty(&self, address: &str, reason: &str) -> Result<()>;
    /// Readiness probe; failures are expected while the bot boots.
    fn drain_status_ok(&self, address: &str) -> bool;
    /// Full drain state, for `sakiot-deploy status`.
    fn drain_status(&self, address: &str) -> Result<DrainStatus>;
}

pub struct TonicAdmin {
    runtime: tokio::runtime::Runtime,
}

impl TonicAdmin {
    pub fn new() -> Result<TonicAdmin> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?;
        Ok(TonicAdmin { runtime })
    }

    fn connect(&self, address: &str) -> Result<AdminClient<tonic::transport::Channel>> {
        let endpoint = Endpoint::from_shared(format!("http://{address}"))
            .with_context(|| format!("invalid gRPC address {address}"))?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3));
        let channel = self
            .runtime
            .block_on(endpoint.connect())
            .with_context(|| format!("failed to connect to FBI Agent at {address}"))?;
        Ok(AdminClient::new(channel))
    }

    fn call(
        &self,
        address: &str,
        reason: &str,
        rpc: impl FnOnce(
            &mut AdminClient<tonic::transport::Channel>,
            DrainRequest,
        ) -> Result<(), tonic::Status>,
    ) -> Result<()> {
        let mut client = self.connect(address)?;
        let request = DrainRequest {
            reason: reason.to_string(),
        };
        rpc(&mut client, request).with_context(|| format!("Admin RPC to {address} failed"))?;
        Ok(())
    }
}

impl AdminApi for TonicAdmin {
    fn start_drain(&self, address: &str, reason: &str) -> Result<()> {
        self.call(address, reason, |client, request| {
            self.runtime.block_on(client.start_drain(request)).map(drop)
        })
    }

    fn cancel_drain(&self, address: &str, reason: &str) -> Result<()> {
        self.call(address, reason, |client, request| {
            self.runtime
                .block_on(client.cancel_drain(request))
                .map(drop)
        })
    }

    fn shutdown_when_empty(&self, address: &str, reason: &str) -> Result<()> {
        self.call(address, reason, |client, request| {
            self.runtime
                .block_on(client.shutdown_when_empty(request))
                .map(drop)
        })
    }

    fn drain_status_ok(&self, address: &str) -> bool {
        let Ok(mut client) = self.connect(address) else {
            return false;
        };
        self.runtime
            .block_on(client.get_drain_status(Empty {}))
            .is_ok()
    }

    fn drain_status(&self, address: &str) -> Result<DrainStatus> {
        let mut client = self.connect(address)?;
        let response = self
            .runtime
            .block_on(client.get_drain_status(Empty {}))
            .with_context(|| format!("GetDrainStatus to {address} failed"))?;
        Ok(response.into_inner())
    }
}

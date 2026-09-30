//! FBI Agent Admin gRPC client, with a 2 s connect timeout and a 3 s limit
//! per call. Every call presents the environment's `FBI_AGENT_REGISTRY_SECRET`,
//! which the agent requires (fbi-agent's `grpc::auth`).

use std::time::Duration;

use anyhow::{Context, Result};
use sakiot_proto::INTERNAL_SECRET_HEADER;
use sakiot_proto::fbi_agent::admin_client::AdminClient;
use sakiot_proto::fbi_agent::{DrainRequest, DrainStatus, Empty};
use tonic::metadata::AsciiMetadataValue;
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
    secret: Option<AsciiMetadataValue>,
}

impl TonicAdmin {
    /// An empty `secret` sends none, which only a debug-build agent accepts.
    pub fn new(secret: &str) -> Result<TonicAdmin> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?;
        let secret = if secret.is_empty() {
            None
        } else {
            Some(
                secret
                    .parse()
                    .context("FBI_AGENT_REGISTRY_SECRET cannot be sent as gRPC metadata")?,
            )
        };
        Ok(TonicAdmin { runtime, secret })
    }

    fn request<T>(&self, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        if let Some(secret) = &self.secret {
            request
                .metadata_mut()
                .insert(INTERNAL_SECRET_HEADER, secret.clone());
        }
        request
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
            tonic::Request<DrainRequest>,
        ) -> Result<(), tonic::Status>,
    ) -> Result<()> {
        let mut client = self.connect(address)?;
        let request = self.request(DrainRequest {
            reason: reason.to_string(),
        });
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
            .block_on(client.get_drain_status(self.request(Empty {})))
            .is_ok()
    }

    fn drain_status(&self, address: &str) -> Result<DrainStatus> {
        let mut client = self.connect(address)?;
        let response = self
            .runtime
            .block_on(client.get_drain_status(self.request(Empty {})))
            .with_context(|| format!("GetDrainStatus to {address} failed"))?;
        Ok(response.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_admin_request_carries_the_configured_secret() {
        let admin = TonicAdmin::new("s3cret").unwrap();
        let request = admin.request(Empty {});
        assert_eq!(
            request.metadata().get(INTERNAL_SECRET_HEADER).unwrap(),
            "s3cret"
        );

        let open = TonicAdmin::new("").unwrap();
        assert!(
            open.request(Empty {})
                .metadata()
                .get(INTERNAL_SECRET_HEADER)
                .is_none()
        );
    }

    #[test]
    fn a_secret_that_cannot_be_metadata_is_rejected_up_front() {
        assert!(TonicAdmin::new("line\nbreak").is_err());
    }
}

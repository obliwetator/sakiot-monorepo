use crate::Custom;
use std::net::{AddrParseError, SocketAddr};
use tokio::sync::watch;
use tonic::transport::Server;
use tracing::info;

pub mod proto {
    pub use sakiot_proto::fbi_agent::*;
}

mod admin;
mod dashboard;
mod jammer;
mod snapshot;

#[derive(Clone)]
pub struct FbiAgentGrpc {
    data_cache: Custom,
}

impl FbiAgentGrpc {
    pub fn new(data_cache: Custom) -> Self {
        Self { data_cache }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GrpcServerError {
    #[error("invalid gRPC address")]
    InvalidAddress(#[source] AddrParseError),
    #[error("gRPC server failed")]
    Serve(#[source] tonic::transport::Error),
}

pub(crate) fn spawn_server(
    data_cache: Custom,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<Result<(), GrpcServerError>> {
    tokio::spawn(async move {
        let addr: SocketAddr = crate::config::grpc_addr()
            .parse()
            .map_err(GrpcServerError::InvalidAddress)?;

        let jammer = FbiAgentGrpc::new(data_cache);
        info!("gRPC server listening on {}", addr);

        Server::builder()
            .add_service(proto::jammer_server::JammerServer::new(jammer.clone()))
            .add_service(proto::admin_server::AdminServer::new(jammer.clone()))
            .add_service(proto::dashboard_server::DashboardServer::new(jammer))
            .serve_with_shutdown(addr, async move {
                let mut rx = shutdown_rx;
                while !*rx.borrow() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .map_err(GrpcServerError::Serve)
    })
}

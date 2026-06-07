use crate::Custom;
use tokio::sync::watch;
use tonic::transport::Server;
use tracing::{error, info};

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

pub(crate) fn spawn_server(
    data_cache: Custom,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let addr = match crate::config::grpc_addr().parse() {
            Ok(addr) => addr,
            Err(err) => {
                error!("Invalid gRPC address: {}", err);
                return;
            }
        };

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
            .unwrap_or_else(|err| error!("gRPC server failed: {}", err));
    })
}

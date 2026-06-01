use std::sync::atomic::Ordering;

use tonic::{Request, Response, Status};
use tracing::info;

use super::FbiAgentGrpc;
use super::proto::admin_server::Admin;
use super::proto::{DrainRequest, DrainStatus, Empty};

#[tonic::async_trait]
impl Admin for FbiAgentGrpc {
    async fn start_drain(
        &self,
        request: Request<DrainRequest>,
    ) -> Result<Response<DrainStatus>, Status> {
        let reason = request.into_inner().reason;
        info!(reason = %reason, "admin requested drain");
        self.data_cache.runtime.start_drain(false);
        self.clear_voice_presence_metrics().await;
        crate::deployment::heartbeat_instance_and_leases(
            &self.data_cache.pool,
            &self.data_cache.runtime,
        )
        .await;
        Ok(Response::new(self.status("drain started").await))
    }

    async fn get_drain_status(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<DrainStatus>, Status> {
        Ok(Response::new(self.status("ok").await))
    }

    async fn shutdown_when_empty(
        &self,
        request: Request<DrainRequest>,
    ) -> Result<Response<DrainStatus>, Status> {
        let reason = request.into_inner().reason;
        info!(reason = %reason, "admin requested shutdown when empty");
        self.data_cache.runtime.start_drain(true);
        self.clear_voice_presence_metrics().await;
        crate::deployment::heartbeat_instance_and_leases(
            &self.data_cache.pool,
            &self.data_cache.runtime,
        )
        .await;
        Ok(Response::new(
            self.status("shutdown when empty requested").await,
        ))
    }

    async fn force_shutdown(
        &self,
        request: Request<DrainRequest>,
    ) -> Result<Response<DrainStatus>, Status> {
        let reason = request.into_inner().reason;
        info!(reason = %reason, "admin requested force shutdown");
        self.data_cache.runtime.force_shutdown();
        self.clear_voice_presence_metrics().await;
        crate::deployment::heartbeat_instance_and_leases(
            &self.data_cache.pool,
            &self.data_cache.runtime,
        )
        .await;
        Ok(Response::new(self.status("force shutdown requested").await))
    }
}

impl FbiAgentGrpc {
    async fn clear_voice_presence_metrics(&self) {
        let data = self.data_cache.data.read().await;
        if let Some(metrics) = data.get::<crate::BotMetricsKey>() {
            metrics.clear_voice_presence();
        }
    }

    async fn status(&self, message: &str) -> DrainStatus {
        let active_voice_connections = {
            let data = self.data_cache.data.read().await;
            data.get::<songbird::SongbirdKey>()
                .map(|manager| manager.iter().count() as u32)
                .or_else(|| {
                    data.get::<crate::BotMetricsKey>()
                        .map(|metrics| metrics.active_voice_connections.load(Ordering::Relaxed))
                })
                .unwrap_or(0)
        };
        let active_recordings = {
            let data = self.data_cache.data.read().await;
            data.get::<crate::BotMetricsKey>()
                .map(|metrics| metrics.active_recordings.load(Ordering::Relaxed) as u64)
                .unwrap_or(0)
        };

        DrainStatus {
            instance_id: self.data_cache.runtime.config().instance_id.clone(),
            role: self.data_cache.runtime.role().as_str().to_string(),
            draining: self.data_cache.runtime.is_draining(),
            shutdown_when_empty: self.data_cache.runtime.shutdown_when_empty(),
            drain_timeout_seconds: self.data_cache.runtime.config().drain_timeout.as_secs(),
            active_voice_connections,
            message: message.to_string(),
            drain_age_seconds: self.data_cache.runtime.drain_age_seconds(),
            force_shutdown: self.data_cache.runtime.force_shutdown_requested(),
            voice_state: if active_voice_connections > 0 {
                "connected".to_string()
            } else {
                "empty".to_string()
            },
            active_recordings,
        }
    }
}

use std::sync::atomic::Ordering;

use tonic::{Request, Response, Status};
use tracing::info;

use super::FbiAgentGrpc;
use super::proto::admin_server::Admin;
use super::proto::{BotRole as ProtoBotRole, DrainRequest, DrainStatus, Empty, VoicePresence};
use crate::runtime::BotRole as RuntimeBotRole;

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
        .await
        .map_err(|err| Status::internal(format!("database error: {err}")))?;
        Ok(Response::new(self.status("drain started").await))
    }

    async fn cancel_drain(
        &self,
        request: Request<DrainRequest>,
    ) -> Result<Response<DrainStatus>, Status> {
        let reason = request.into_inner().reason;
        info!(reason = %reason, "admin requested drain cancellation");
        if !self.data_cache.runtime.cancel_drain() {
            return Err(Status::failed_precondition(
                "force shutdown cannot be cancelled",
            ));
        }
        crate::deployment::heartbeat_instance_and_leases(
            &self.data_cache.pool,
            &self.data_cache.runtime,
        )
        .await
        .map_err(|err| Status::internal(format!("database error: {err}")))?;
        Ok(Response::new(self.status("drain cancelled").await))
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
        .await
        .map_err(|err| Status::internal(format!("database error: {err}")))?;
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
        .await
        .map_err(|err| Status::internal(format!("database error: {err}")))?;
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

        let runtime_role = self.data_cache.runtime.role();
        let voice_presence = if active_voice_connections > 0 {
            VoicePresence::Connected
        } else {
            VoicePresence::Empty
        };
        #[allow(deprecated)]
        let status = DrainStatus {
            instance_id: self.data_cache.runtime.config().instance_id.clone(),
            role: runtime_role.as_str().to_string(),
            draining: self.data_cache.runtime.is_draining(),
            shutdown_when_empty: self.data_cache.runtime.shutdown_when_empty(),
            drain_timeout_seconds: self.data_cache.runtime.config().drain_timeout.as_secs(),
            active_voice_connections,
            message: message.to_string(),
            drain_age_seconds: self.data_cache.runtime.drain_age_seconds(),
            force_shutdown: self.data_cache.runtime.force_shutdown_requested(),
            voice_state: legacy_voice_presence(voice_presence).to_string(),
            active_recordings,
            bot_role: proto_bot_role(runtime_role) as i32,
            voice_presence: voice_presence as i32,
        };
        status
    }
}

fn proto_bot_role(role: RuntimeBotRole) -> ProtoBotRole {
    match role {
        RuntimeBotRole::Active => ProtoBotRole::Active,
        RuntimeBotRole::Drain => ProtoBotRole::Drain,
    }
}

fn legacy_voice_presence(presence: VoicePresence) -> &'static str {
    match presence {
        VoicePresence::Unspecified | VoicePresence::Empty => "empty",
        VoicePresence::Connected => "connected",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProtoBotRole, RuntimeBotRole, VoicePresence, legacy_voice_presence, proto_bot_role,
    };

    #[test]
    fn runtime_roles_have_typed_grpc_values() {
        assert_eq!(proto_bot_role(RuntimeBotRole::Active), ProtoBotRole::Active);
        assert_eq!(proto_bot_role(RuntimeBotRole::Drain), ProtoBotRole::Drain);
        assert_eq!(legacy_voice_presence(VoicePresence::Empty), "empty");
        assert_eq!(legacy_voice_presence(VoicePresence::Connected), "connected");
    }
}

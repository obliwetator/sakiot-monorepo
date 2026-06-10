use std::sync::Arc;

use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use songbird::{CoreEvent, SongbirdKey};
use sqlx::{Pool, Postgres};
use tracing::{error, info, warn};

use crate::{BotMetricsKey, events::voice_receiver::Receiver};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VoiceConnectOutcome {
    Joined,
    Rejoined,
    Switched,
    AlreadyInChannel,
    SkippedDraining,
    SkippedLeaseOwned { owner: String },
    VoiceSystemMissing,
    Failed(String),
}

impl VoiceConnectOutcome {
    pub fn user_message(&self) -> String {
        match self {
            Self::Joined => "Joined your voice channel.".to_string(),
            Self::Rejoined => "Rejoined your voice channel.".to_string(),
            Self::Switched => "Switched to your voice channel.".to_string(),
            Self::AlreadyInChannel => "Already in your voice channel.".to_string(),
            Self::SkippedDraining => {
                "This bot instance is draining and will not join voice.".to_string()
            }
            Self::SkippedLeaseOwned { .. } => {
                "Another bot instance is already handling voice for this server.".to_string()
            }
            Self::VoiceSystemMissing => "Voice system is not configured.".to_string(),
            Self::Failed(err) => format!("Failed to join voice: {}", err),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VoiceDisconnectOutcome {
    Disconnected,
    NotConnected,
    VoiceSystemMissing,
    Failed(String),
}

impl VoiceDisconnectOutcome {
    pub fn action_response_message(&self) -> String {
        match self {
            Self::Disconnected => "Successfully disconnected from voice".to_string(),
            Self::NotConnected => "Bot is not in a voice channel in this guild".to_string(),
            Self::VoiceSystemMissing => "Voice system is not configured".to_string(),
            Self::Failed(err) => format!("Failed to disconnect: {}", err),
        }
    }

    pub fn success(&self) -> bool {
        matches!(self, Self::Disconnected)
    }
}

pub async fn disconnect_voice_channel(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> VoiceDisconnectOutcome {
    let (manager, runtime) = {
        let data_read = data.read().await;
        (
            data_read.get::<SongbirdKey>().cloned(),
            data_read.get::<crate::runtime::RuntimeStateKey>().cloned(),
        )
    };

    let Some(manager) = manager else {
        error!("Songbird manager missing while leaving voice channel");
        refresh_active_voice_connection_gauge(data, None).await;
        return VoiceDisconnectOutcome::VoiceSystemMissing;
    };

    let existed = manager.get(guild_id).is_some();
    if !existed {
        refresh_active_voice_connection_gauge(data, Some(&manager)).await;
        return VoiceDisconnectOutcome::NotConnected;
    }

    match manager.remove(guild_id).await {
        Ok(()) => {
            if let Some(runtime) = runtime
                && let Err(err) =
                    crate::deployment::release_voice_session(pool, &runtime, guild_id).await
            {
                warn!(
                    guild_id = guild_id.get(),
                    "voice lease release failed after leave: {}", err
                );
            }
            refresh_active_voice_connection_gauge(data, Some(&manager)).await;
            VoiceDisconnectOutcome::Disconnected
        }
        Err(err) => {
            refresh_active_voice_connection_gauge(data, Some(&manager)).await;
            VoiceDisconnectOutcome::Failed(err.to_string())
        }
    }
}

pub async fn connect_to_voice_channel(
    pool: Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> VoiceConnectOutcome {
    if crate::runtime::is_draining_ctx(ctx).await {
        info!(
            guild_id = guild_id.get(),
            channel_id = channel_id.get(),
            "skipping voice connect while instance is draining"
        );
        return VoiceConnectOutcome::SkippedDraining;
    }

    let Some(manager) = songbird::get(ctx).await else {
        error!("Songbird manager missing while connecting to voice channel");
        refresh_active_voice_connection_gauge(&ctx.data, None).await;
        return VoiceConnectOutcome::VoiceSystemMissing;
    };
    let manager = manager.clone();

    if let Some(runtime) = crate::runtime::state_from_ctx(ctx).await {
        match crate::deployment::active_lease_owner(&pool, guild_id).await {
            Ok(Some(owner)) if owner != runtime.config().instance_id => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    owner = %owner,
                    "skipping voice connect because another instance owns lease"
                );
                return VoiceConnectOutcome::SkippedLeaseOwned { owner };
            }
            Ok(_) => {}
            Err(err) => {
                warn!(
                    guild_id = guild_id.get(),
                    "failed to inspect voice lease before join: {}", err
                );
            }
        }
    }

    if let Some(arc_call) = manager.get(guild_id) {
        let current = arc_call.lock().await.current_channel();

        match current {
            Some(ch) if ch.0.get() == channel_id.get() => {
                refresh_active_voice_connection_gauge(&ctx.data, Some(&manager)).await;
                VoiceConnectOutcome::AlreadyInChannel
            }
            Some(ch) => switch_channel(pool, manager, guild_id, channel_id, ctx, ch.0.get()).await,
            None => {
                info!("Call exists but disconnected, rejoining");
                join_ch(
                    pool,
                    manager,
                    guild_id,
                    channel_id,
                    ctx,
                    JoinMode::RejoinDisconnected,
                )
                .await
            }
        }
    } else {
        join_ch(pool, manager, guild_id, channel_id, ctx, JoinMode::Fresh).await
    }
}

enum JoinMode {
    Fresh,
    RejoinDisconnected,
    SwitchFresh { _old_channel: u64 },
}

async fn switch_channel(
    pool: Pool<Postgres>,
    manager: Arc<songbird::Songbird>,
    guild_id: GuildId,
    channel_id: ChannelId,
    ctx: &Context,
    old_channel: u64,
) -> VoiceConnectOutcome {
    match manager.remove(guild_id).await {
        Ok(()) => {
            if let Some(runtime) = crate::runtime::state_from_ctx(ctx).await
                && let Err(err) =
                    crate::deployment::release_voice_session(&pool, &runtime, guild_id).await
            {
                warn!(
                    guild_id = guild_id.get(),
                    "voice lease release failed before switch: {}", err
                );
            }
            refresh_active_voice_connection_gauge(&ctx.data, Some(&manager)).await;
            join_ch(
                pool,
                manager,
                guild_id,
                channel_id,
                ctx,
                JoinMode::SwitchFresh {
                    _old_channel: old_channel,
                },
            )
            .await
        }
        Err(err) => {
            refresh_active_voice_connection_gauge(&ctx.data, Some(&manager)).await;
            VoiceConnectOutcome::Failed(err.to_string())
        }
    }
}

async fn join_ch(
    pool: Pool<Postgres>,
    manager: Arc<songbird::Songbird>,
    guild_id: GuildId,
    channel_id: ChannelId,
    ctx: &Context,
    mode: JoinMode,
) -> VoiceConnectOutcome {
    let runtime = crate::runtime::state_from_ctx(ctx).await;
    let claimed_lease = if let Some(runtime) = &runtime {
        match crate::deployment::claim_voice_session(&pool, runtime, guild_id, channel_id).await {
            Ok(crate::deployment::VoiceLeaseClaim::Claimed) => true,
            Ok(crate::deployment::VoiceLeaseClaim::OwnedByOther(owner)) => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    owner = %owner,
                    "skipping voice connect because another instance won lease claim"
                );
                return VoiceConnectOutcome::SkippedLeaseOwned { owner };
            }
            Err(err) => {
                error!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    "voice lease claim failed: {}",
                    err
                );
                return VoiceConnectOutcome::Failed(format!("voice lease claim failed: {}", err));
            }
        }
    } else {
        false
    };

    let handler_lock = manager.get_or_insert(guild_id);
    let result = {
        let mut handler = handler_lock.lock().await;
        if matches!(mode, JoinMode::Fresh | JoinMode::SwitchFresh { .. }) {
            register_voice_receiver(&mut handler, pool.clone(), ctx, guild_id, channel_id, true)
                .await;
        }
        handler.join(channel_id).await
    };

    match result {
        Ok(join) => match join.await {
            Ok(()) => {
                refresh_active_voice_connection_gauge(&ctx.data, Some(&manager)).await;
                match mode {
                    JoinMode::Fresh => VoiceConnectOutcome::Joined,
                    JoinMode::RejoinDisconnected => VoiceConnectOutcome::Rejoined,
                    JoinMode::SwitchFresh { .. } => VoiceConnectOutcome::Switched,
                }
            }
            Err(err) => {
                error!("cannot join channel {}: {}", channel_id, err);
                if claimed_lease
                    && let Some(runtime) = &runtime
                    && let Err(release_err) =
                        crate::deployment::release_voice_session(&pool, runtime, guild_id).await
                {
                    warn!(
                        guild_id = guild_id.get(),
                        "voice lease release failed after join error: {}", release_err
                    );
                }
                cleanup_failed_join(&ctx.data, &manager, guild_id).await;
                VoiceConnectOutcome::Failed(err.to_string())
            }
        },
        Err(err) => {
            error!("cannot join channel {}: {}", channel_id, err);
            if claimed_lease
                && let Some(runtime) = &runtime
                && let Err(release_err) =
                    crate::deployment::release_voice_session(&pool, runtime, guild_id).await
            {
                warn!(
                    guild_id = guild_id.get(),
                    "voice lease release failed after join error: {}", release_err
                );
            }
            cleanup_failed_join(&ctx.data, &manager, guild_id).await;
            VoiceConnectOutcome::Failed(err.to_string())
        }
    }
}

async fn cleanup_failed_join(
    data: &Arc<RwLock<TypeMap>>,
    manager: &Arc<songbird::Songbird>,
    guild_id: GuildId,
) {
    if let Err(remove_err) = manager.remove(guild_id).await {
        error!("failed to clean up failed voice join: {}", remove_err);
    }
    refresh_active_voice_connection_gauge(data, Some(manager)).await;
}

async fn register_voice_receiver(
    handler: &mut songbird::Call,
    pool: Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    reset_existing_handlers: bool,
) {
    if reset_existing_handlers {
        handler.remove_all_global_events();
    }

    let metrics = {
        let data_read = ctx.data.read().await;
        let Some(m) = data_read.get::<BotMetricsKey>() else {
            error!("BotMetrics missing while joining voice channel");
            return;
        };
        m.clone()
    };

    let ctx1 = Arc::new(ctx.clone());
    let receiver = Receiver::new(pool, ctx1, guild_id, channel_id, metrics).await;

    handler.add_global_event(CoreEvent::SpeakingStateUpdate.into(), receiver.clone());
    handler.add_global_event(CoreEvent::VoiceTick.into(), receiver.clone());
    handler.add_global_event(CoreEvent::RtcpPacket.into(), receiver.clone());
    handler.add_global_event(CoreEvent::ClientDisconnect.into(), receiver.clone());
    handler.add_global_event(CoreEvent::DriverConnect.into(), receiver.clone());
    handler.add_global_event(CoreEvent::DriverReconnect.into(), receiver.clone());
    handler.add_global_event(CoreEvent::DriverDisconnect.into(), receiver.clone());
}

pub async fn refresh_active_voice_connection_gauge(
    data: &Arc<RwLock<TypeMap>>,
    manager: Option<&Arc<songbird::Songbird>>,
) {
    let count = manager
        .map(|manager| manager.iter().count() as u32)
        .unwrap_or(0);
    let data_read = data.read().await;
    if let Some(metrics) = data_read.get::<BotMetricsKey>() {
        metrics
            .active_voice_connections
            .store(count, std::sync::atomic::Ordering::Relaxed);
        let _ = metrics.update_tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::{VoiceConnectOutcome, VoiceDisconnectOutcome};

    #[test]
    fn connect_outcomes_have_user_safe_messages() {
        assert_eq!(
            VoiceConnectOutcome::AlreadyInChannel.user_message(),
            "Already in your voice channel."
        );
        assert_eq!(
            VoiceConnectOutcome::SkippedLeaseOwned {
                owner: "other".to_string()
            }
            .user_message(),
            "Another bot instance is already handling voice for this server."
        );
        assert_eq!(
            VoiceConnectOutcome::Failed("boom".to_string()).user_message(),
            "Failed to join voice: boom"
        );
    }

    #[test]
    fn disconnect_success_only_for_real_disconnect() {
        assert!(VoiceDisconnectOutcome::Disconnected.success());
        assert!(!VoiceDisconnectOutcome::NotConnected.success());
        assert!(!VoiceDisconnectOutcome::VoiceSystemMissing.success());
        assert!(!VoiceDisconnectOutcome::Failed("boom".to_string()).success());
    }
}

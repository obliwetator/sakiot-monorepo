use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use songbird::{CoreEvent, SongbirdKey};
use sqlx::{Pool, Postgres};
use tracing::{error, info, warn};

use crate::cast::ToI64;
use crate::{BotMetricsKey, events::voice_receiver::Receiver};

static FALLBACK_OPERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

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

    pub(crate) fn should_retry_routing(&self) -> bool {
        matches!(self, Self::Failed(_))
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

#[derive(Clone, Debug)]
pub(crate) struct VoiceOperation {
    pub(crate) operation_id: String,
    pub(crate) trigger: String,
    pub(crate) started_at_ms: i64,
    pub(crate) population_snapshot: serde_json::Value,
    pub(crate) has_afk_channel: bool,
    pub(crate) pending_cap_seconds: i64,
}

impl VoiceOperation {
    pub(crate) fn ad_hoc(ctx: &Context, guild_id: GuildId, trigger: &str) -> Self {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let counter = FALLBACK_OPERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
        Self {
            operation_id: format!(
                "adhoc-{}-{}-{}-{}",
                ctx.cache.current_user().id.get(),
                guild_id.get(),
                started_at_ms,
                counter
            ),
            trigger: trigger.to_string(),
            started_at_ms,
            population_snapshot: serde_json::json!([]),
            has_afk_channel: false,
            pending_cap_seconds: crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS,
        }
    }
}

#[derive(Debug)]
pub(crate) struct VoiceTeardownReport {
    pub(crate) manager_missing: bool,
    pub(crate) had_call: bool,
    pub(crate) connected_after: bool,
    pub(crate) remove_error: Option<String>,
}

pub async fn disconnect_voice_channel(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> VoiceDisconnectOutcome {
    let report = teardown_voice_session(data, pool, guild_id).await;

    if report.manager_missing {
        return VoiceDisconnectOutcome::VoiceSystemMissing;
    }
    if !report.had_call {
        return VoiceDisconnectOutcome::NotConnected;
    }
    if !report.connected_after {
        return VoiceDisconnectOutcome::Disconnected;
    }

    VoiceDisconnectOutcome::Failed(
        report
            .remove_error
            .unwrap_or_else(|| "voice connection remained locally connected".to_string()),
    )
}

pub(crate) async fn teardown_voice_session(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> VoiceTeardownReport {
    teardown_voice_session_with_operation(data, pool, guild_id, None).await
}

pub(crate) async fn teardown_voice_session_with_operation(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    operation: Option<&VoiceOperation>,
) -> VoiceTeardownReport {
    let started_at_ms = operation
        .map(|operation| operation.started_at_ms)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let (manager, runtime) = {
        let data_read = data.read().await;
        (
            data_read.get::<SongbirdKey>().cloned(),
            data_read.get::<crate::runtime::RuntimeStateKey>().cloned(),
        )
    };

    let Some(manager) = manager else {
        error!("Songbird manager missing while tearing down voice session");
        release_disconnected_lease(pool, runtime.as_deref(), guild_id).await;
        refresh_active_voice_connection_gauge(data, None).await;
        return VoiceTeardownReport {
            manager_missing: true,
            had_call: false,
            connected_after: false,
            remove_error: None,
        };
    };

    let call_before = manager.get(guild_id);
    let from_channel = current_songbird_channel(call_before.clone()).await;
    let had_call = call_before.is_some();
    let remove_error = if had_call {
        manager.remove(guild_id).await.err().map(|err| {
            warn!(
                guild_id = guild_id.get(),
                "Songbird remove failed during voice teardown: {}", err
            );
            err.to_string()
        })
    } else {
        None
    };
    let connected_after = current_channel_is_some(manager.get(guild_id)).await;
    if !connected_after {
        release_disconnected_lease(pool, runtime.as_deref(), guild_id).await;
        crate::events::voice_receiver::notify_voice_session_ended(
            data,
            guild_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .await;
    }

    refresh_active_voice_connection_gauge(data, Some(&manager)).await;
    let completed_at_ms = chrono::Utc::now().timestamp_millis();
    let owner = runtime
        .as_ref()
        .map(|runtime| runtime.config().instance_id.as_str());
    let generated_operation_id = format!(
        "disconnect-{}-{}-{}",
        guild_id.get(),
        started_at_ms,
        FALLBACK_OPERATION_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let operation_id = operation
        .map(|operation| operation.operation_id.as_str())
        .unwrap_or(&generated_operation_id);
    let trigger = operation
        .map(|operation| operation.trigger.as_str())
        .unwrap_or("disconnect");
    let outcome = if !had_call {
        "not_connected"
    } else if connected_after {
        "disconnect_failed"
    } else {
        "disconnected"
    };
    let empty_snapshot = serde_json::json!([]);
    let population_snapshot = operation
        .map(|operation| &operation.population_snapshot)
        .unwrap_or(&empty_snapshot);
    if let Err(err) = crate::database::voice_events::insert_voice_connection_event(
        pool,
        crate::database::voice_events::VoiceConnectionEvent {
            operation_id,
            guild_id: guild_id.to_i64(),
            owner_instance_id: owner,
            release_id: release_id().as_deref(),
            trigger,
            started_at_ms,
            completed_at_ms,
            from_channel_id: from_channel.map(ToI64::to_i64),
            to_channel_id: None,
            population_snapshot,
            outcome,
            error: remove_error.as_deref(),
            fallback_outcome: None,
            fallback_error: None,
        },
    )
    .await
    {
        warn!("failed to record voice disconnect event: {}", err);
    }

    VoiceTeardownReport {
        manager_missing: false,
        had_call,
        connected_after,
        remove_error,
    }
}

async fn current_channel_is_some(call: Option<Arc<tokio::sync::Mutex<songbird::Call>>>) -> bool {
    current_songbird_channel(call).await.is_some()
}

async fn current_songbird_channel(
    call: Option<Arc<tokio::sync::Mutex<songbird::Call>>>,
) -> Option<ChannelId> {
    match call {
        Some(call) => call
            .lock()
            .await
            .current_channel()
            .map(|channel| ChannelId::new(channel.0.get())),
        None => None,
    }
}

async fn release_disconnected_lease(
    pool: &Pool<Postgres>,
    runtime: Option<&crate::runtime::RuntimeState>,
    guild_id: GuildId,
) {
    let Some(runtime) = runtime else {
        return;
    };

    if let Err(err) = crate::deployment::release_voice_session(pool, runtime, guild_id).await {
        warn!(
            guild_id = guild_id.get(),
            "voice lease release failed after local disconnect: {}", err
        );
    }
}

pub async fn connect_to_voice_channel(
    pool: Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> VoiceConnectOutcome {
    let operation = VoiceOperation::ad_hoc(ctx, guild_id, "manual");
    connect_to_voice_channel_with_operation(pool, ctx, guild_id, channel_id, operation).await
}

pub(crate) async fn connect_to_voice_channel_with_operation(
    pool: Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    operation: VoiceOperation,
) -> VoiceConnectOutcome {
    if crate::runtime::is_draining_ctx(ctx).await {
        info!(
            guild_id = guild_id.get(),
            channel_id = channel_id.get(),
            "skipping voice connect while instance is draining"
        );
        let report =
            ConnectReport::simple(VoiceConnectOutcome::SkippedDraining, "skipped_draining");
        record_connect_event(&pool, ctx, guild_id, None, channel_id, &operation, &report).await;
        return report.outcome;
    }

    let Some(manager) = songbird::get(ctx).await else {
        error!("Songbird manager missing while connecting to voice channel");
        refresh_active_voice_connection_gauge(&ctx.data, None).await;
        let report = ConnectReport::simple(
            VoiceConnectOutcome::VoiceSystemMissing,
            "voice_system_missing",
        );
        record_connect_event(&pool, ctx, guild_id, None, channel_id, &operation, &report).await;
        return report.outcome;
    };
    let manager = manager.clone();

    let runtime = crate::runtime::state_from_ctx(ctx).await;
    if let Some(runtime) = &runtime {
        match crate::deployment::active_lease_owner(&pool, guild_id).await {
            Ok(Some(owner)) if owner != runtime.config().instance_id => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    owner = %owner,
                    "skipping voice connect because another instance owns lease"
                );
                let report = ConnectReport::simple(
                    VoiceConnectOutcome::SkippedLeaseOwned {
                        owner: owner.clone(),
                    },
                    "skipped_lease_owned",
                );
                record_connect_event(
                    &pool,
                    ctx,
                    guild_id,
                    actual_bot_channel(ctx, guild_id, manager.get(guild_id)).await,
                    channel_id,
                    &operation,
                    &report,
                )
                .await;
                return report.outcome;
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

    let from_channel = actual_bot_channel(ctx, guild_id, manager.get(guild_id)).await;
    let connection = ConnectionContext {
        pool: &pool,
        manager: &manager,
        guild_id,
        ctx,
        runtime: runtime.as_deref(),
        operation: &operation,
    };
    let report = if from_channel == Some(channel_id) {
        refresh_active_voice_connection_gauge(&ctx.data, Some(&manager)).await;
        crate::events::voice_receiver::complete_handoff(
            ctx,
            guild_id,
            channel_id,
            chrono::Utc::now().timestamp_millis(),
            operation.has_afk_channel,
            operation.pending_cap_seconds,
        )
        .await;
        ConnectReport::simple(VoiceConnectOutcome::AlreadyInChannel, "already_in_channel")
    } else if let Some(old_channel) = from_channel {
        switch_channel(&connection, old_channel, channel_id).await
    } else {
        join_fresh(&connection, channel_id, manager.get(guild_id).is_some()).await
    };

    record_connect_event(
        &pool,
        ctx,
        guild_id,
        from_channel,
        channel_id,
        &operation,
        &report,
    )
    .await;
    report.outcome
}

#[derive(Clone, Copy)]
struct ConnectionContext<'a> {
    pool: &'a Pool<Postgres>,
    manager: &'a Arc<songbird::Songbird>,
    guild_id: GuildId,
    ctx: &'a Context,
    runtime: Option<&'a crate::runtime::RuntimeState>,
    operation: &'a VoiceOperation,
}

#[derive(Debug)]
struct ConnectReport {
    outcome: VoiceConnectOutcome,
    audit_outcome: &'static str,
    error: Option<String>,
    fallback_outcome: Option<&'static str>,
    fallback_error: Option<String>,
}

impl ConnectReport {
    fn simple(outcome: VoiceConnectOutcome, audit_outcome: &'static str) -> Self {
        Self {
            outcome,
            audit_outcome,
            error: None,
            fallback_outcome: None,
            fallback_error: None,
        }
    }

    fn failed(error: String, audit_outcome: &'static str) -> Self {
        Self {
            outcome: VoiceConnectOutcome::Failed(error.clone()),
            audit_outcome,
            error: Some(error),
            fallback_outcome: None,
            fallback_error: None,
        }
    }
}

async fn switch_channel(
    connection: &ConnectionContext<'_>,
    old_channel: ChannelId,
    target_channel: ChannelId,
) -> ConnectReport {
    let ConnectionContext {
        pool,
        manager,
        guild_id,
        ctx,
        runtime,
        operation,
    } = *connection;
    crate::events::voice_receiver::begin_handoff(
        ctx,
        guild_id,
        old_channel,
        Some(target_channel),
        operation.has_afk_channel,
        operation.pending_cap_seconds,
    )
    .await;

    let Some(call) = manager.get(guild_id) else {
        crate::events::voice_receiver::cancel_handoff(ctx, guild_id).await;
        return ConnectReport::failed(
            "voice call disappeared before handoff".to_string(),
            "switch_failed",
        );
    };

    match issue_join(&call, target_channel).await {
        Ok(()) => {
            update_lease_after_handoff(pool, runtime, guild_id, target_channel).await;
            crate::events::voice_receiver::complete_handoff(
                ctx,
                guild_id,
                target_channel,
                chrono::Utc::now().timestamp_millis(),
                operation.has_afk_channel,
                operation.pending_cap_seconds,
            )
            .await;
            refresh_active_voice_connection_gauge(&ctx.data, Some(manager)).await;
            ConnectReport::simple(VoiceConnectOutcome::Switched, "switched")
        }
        Err(target_error) => {
            warn!(
                guild_id = guild_id.get(),
                from_channel_id = old_channel.get(),
                to_channel_id = target_channel.get(),
                "target voice handoff failed: {}",
                target_error
            );
            let actual = actual_bot_channel(ctx, guild_id, Some(call.clone())).await;
            if actual == Some(target_channel) {
                update_lease_after_handoff(pool, runtime, guild_id, target_channel).await;
                crate::events::voice_receiver::complete_handoff(
                    ctx,
                    guild_id,
                    target_channel,
                    chrono::Utc::now().timestamp_millis(),
                    operation.has_afk_channel,
                    operation.pending_cap_seconds,
                )
                .await;
                return ConnectReport {
                    outcome: VoiceConnectOutcome::Switched,
                    audit_outcome: "switched_after_join_error",
                    error: Some(target_error),
                    fallback_outcome: Some("not_needed_target_connected"),
                    fallback_error: None,
                };
            }

            let previous_occupied = cached_human_member_count(ctx, guild_id, old_channel)
                .map(|count| count > 0)
                .unwrap_or(true);
            if previous_occupied {
                crate::events::voice_receiver::begin_handoff(
                    ctx,
                    guild_id,
                    actual.unwrap_or(target_channel),
                    Some(old_channel),
                    operation.has_afk_channel,
                    operation.pending_cap_seconds,
                )
                .await;
                match issue_join(&call, old_channel).await {
                    Ok(()) => {
                        update_lease_after_handoff(pool, runtime, guild_id, old_channel).await;
                        crate::events::voice_receiver::complete_handoff(
                            ctx,
                            guild_id,
                            old_channel,
                            chrono::Utc::now().timestamp_millis(),
                            operation.has_afk_channel,
                            operation.pending_cap_seconds,
                        )
                        .await;
                        refresh_active_voice_connection_gauge(&ctx.data, Some(manager)).await;
                        return ConnectReport {
                            outcome: VoiceConnectOutcome::Failed(format!(
                                "target handoff failed: {target_error}; previous channel restored"
                            )),
                            audit_outcome: "switch_failed_fallback_succeeded",
                            error: Some(target_error),
                            fallback_outcome: Some("rejoined_previous"),
                            fallback_error: None,
                        };
                    }
                    Err(fallback_error) => {
                        let actual_after =
                            actual_bot_channel(ctx, guild_id, Some(call.clone())).await;
                        retain_or_release_after_failure(
                            pool,
                            manager,
                            ctx,
                            runtime,
                            guild_id,
                            actual_after,
                            operation,
                        )
                        .await;
                        return ConnectReport {
                            outcome: VoiceConnectOutcome::Failed(format!(
                                "target handoff failed: {target_error}; fallback failed: {fallback_error}"
                            )),
                            audit_outcome: "switch_and_fallback_failed",
                            error: Some(target_error),
                            fallback_outcome: Some("failed"),
                            fallback_error: Some(fallback_error),
                        };
                    }
                }
            }

            crate::events::voice_receiver::cancel_handoff(ctx, guild_id).await;
            retain_or_release_after_failure(
                pool, manager, ctx, runtime, guild_id, actual, operation,
            )
            .await;
            ConnectReport {
                outcome: VoiceConnectOutcome::Failed(format!(
                    "target handoff failed: {target_error}"
                )),
                audit_outcome: "switch_failed_previous_empty",
                error: Some(target_error),
                fallback_outcome: Some("skipped_previous_empty"),
                fallback_error: None,
            }
        }
    }
}

async fn retain_or_release_after_failure(
    pool: &Pool<Postgres>,
    manager: &Arc<songbird::Songbird>,
    ctx: &Context,
    runtime: Option<&crate::runtime::RuntimeState>,
    guild_id: GuildId,
    actual_channel: Option<ChannelId>,
    operation: &VoiceOperation,
) {
    if let Some(actual_channel) = actual_channel {
        update_lease_after_handoff(pool, runtime, guild_id, actual_channel).await;
        crate::events::voice_receiver::complete_handoff(
            ctx,
            guild_id,
            actual_channel,
            chrono::Utc::now().timestamp_millis(),
            operation.has_afk_channel,
            operation.pending_cap_seconds,
        )
        .await;
        return;
    }

    if let Some(runtime) = runtime
        && let Err(err) = crate::deployment::release_voice_session(pool, runtime, guild_id).await
    {
        warn!(
            guild_id = guild_id.get(),
            "failed to release stale lease: {}", err
        );
    }
    if let Err(err) = manager.remove(guild_id).await {
        warn!(
            guild_id = guild_id.get(),
            "failed to remove disconnected call: {}", err
        );
    }
    refresh_active_voice_connection_gauge(&ctx.data, Some(manager)).await;
}

async fn join_fresh(
    connection: &ConnectionContext<'_>,
    channel_id: ChannelId,
    rejoin_disconnected: bool,
) -> ConnectReport {
    let ConnectionContext {
        pool,
        manager,
        guild_id,
        ctx,
        runtime,
        operation,
    } = *connection;
    let claimed_lease = if let Some(runtime) = runtime {
        match crate::deployment::claim_voice_session(pool, runtime, guild_id, channel_id).await {
            Ok(crate::deployment::VoiceLeaseClaim::Claimed) => true,
            Ok(crate::deployment::VoiceLeaseClaim::OwnedByOther(owner)) => {
                return ConnectReport::simple(
                    VoiceConnectOutcome::SkippedLeaseOwned { owner },
                    "skipped_lease_owned",
                );
            }
            Err(err) => {
                return ConnectReport::failed(
                    format!("voice lease claim failed: {err}"),
                    "join_failed",
                );
            }
        }
    } else {
        false
    };

    let call = manager.get_or_insert(guild_id);
    {
        let mut handler = call.lock().await;
        register_voice_receiver(&mut handler, pool.clone(), ctx, guild_id, channel_id, true).await;
    }

    match issue_join(&call, channel_id).await {
        Ok(()) => {
            crate::events::voice_receiver::complete_handoff(
                ctx,
                guild_id,
                channel_id,
                chrono::Utc::now().timestamp_millis(),
                operation.has_afk_channel,
                operation.pending_cap_seconds,
            )
            .await;
            refresh_active_voice_connection_gauge(&ctx.data, Some(manager)).await;
            if rejoin_disconnected {
                ConnectReport::simple(VoiceConnectOutcome::Rejoined, "rejoined")
            } else {
                ConnectReport::simple(VoiceConnectOutcome::Joined, "joined")
            }
        }
        Err(err) => {
            if claimed_lease
                && let Some(runtime) = runtime
                && let Err(release_err) =
                    crate::deployment::release_voice_session(pool, runtime, guild_id).await
            {
                warn!(
                    guild_id = guild_id.get(),
                    "voice lease release failed after join error: {}", release_err
                );
            }
            cleanup_failed_fresh_join(&ctx.data, manager, guild_id).await;
            ConnectReport::failed(err, "join_failed")
        }
    }
}

async fn issue_join(
    call: &Arc<tokio::sync::Mutex<songbird::Call>>,
    channel_id: ChannelId,
) -> Result<(), String> {
    let join = {
        let mut handler = call.lock().await;
        handler
            .join(channel_id)
            .await
            .map_err(|err| err.to_string())?
    };
    join.await.map_err(|err| err.to_string())
}

async fn update_lease_after_handoff(
    pool: &Pool<Postgres>,
    runtime: Option<&crate::runtime::RuntimeState>,
    guild_id: GuildId,
    channel_id: ChannelId,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if let Err(err) =
        crate::deployment::update_voice_session_channel(pool, runtime, guild_id, channel_id).await
    {
        warn!(
            guild_id = guild_id.get(),
            channel_id = channel_id.get(),
            "voice lease channel update failed after successful handoff: {}",
            err
        );
    }
}

async fn cleanup_failed_fresh_join(
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

async fn actual_bot_channel(
    ctx: &Context,
    guild_id: GuildId,
    call: Option<Arc<tokio::sync::Mutex<songbird::Call>>>,
) -> Option<ChannelId> {
    if let Some(guild) = ctx.cache.guild(guild_id) {
        return guild
            .voice_states
            .get(&ctx.cache.current_user().id)
            .and_then(|state| state.channel_id);
    }
    let call = call?;
    call.lock()
        .await
        .current_connection()
        .map(|connection| ChannelId::new(connection.channel_id.0.get()))
}

fn cached_human_member_count(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Option<usize> {
    let guild = ctx.cache.guild(guild_id)?;
    let mut count = 0;
    for (user_id, voice_state) in &guild.voice_states {
        if voice_state.channel_id != Some(channel_id) || *user_id == ctx.cache.current_user().id {
            continue;
        }
        let is_bot = voice_state
            .member
            .as_ref()
            .map(|member| member.user.bot)
            .or_else(|| guild.members.get(user_id).map(|member| member.user.bot));
        match is_bot {
            Some(false) => count += 1,
            Some(true) => {}
            None => return None,
        }
    }
    Some(count)
}

async fn record_connect_event(
    pool: &Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    from_channel_id: Option<ChannelId>,
    to_channel_id: ChannelId,
    operation: &VoiceOperation,
    report: &ConnectReport,
) {
    let runtime = crate::runtime::state_from_ctx(ctx).await;
    let release = release_id();
    if let Err(err) = crate::database::voice_events::insert_voice_connection_event(
        pool,
        crate::database::voice_events::VoiceConnectionEvent {
            operation_id: &operation.operation_id,
            guild_id: guild_id.to_i64(),
            owner_instance_id: runtime
                .as_ref()
                .map(|runtime| runtime.config().instance_id.as_str()),
            release_id: release.as_deref(),
            trigger: &operation.trigger,
            started_at_ms: operation.started_at_ms,
            completed_at_ms: chrono::Utc::now().timestamp_millis(),
            from_channel_id: from_channel_id.map(ToI64::to_i64),
            to_channel_id: Some(to_channel_id.to_i64()),
            population_snapshot: &operation.population_snapshot,
            outcome: report.audit_outcome,
            error: report.error.as_deref(),
            fallback_outcome: report.fallback_outcome,
            fallback_error: report.fallback_error.as_deref(),
        },
    )
    .await
    {
        warn!(
            guild_id = guild_id.get(),
            operation_id = operation.operation_id,
            "failed to record voice connection operation: {}",
            err
        );
    }
}

fn release_id() -> Option<String> {
    std::env::var("BOT_RELEASE_ID")
        .or_else(|_| std::env::var("RELEASE_ID"))
        .ok()
}

pub async fn refresh_active_voice_connection_gauge(
    data: &Arc<RwLock<TypeMap>>,
    manager: Option<&Arc<songbird::Songbird>>,
) {
    let count = match manager {
        Some(manager) => connected_voice_connection_count(manager).await,
        None => 0,
    };
    let data_read = data.read().await;
    if let Some(metrics) = data_read.get::<BotMetricsKey>() {
        metrics
            .active_voice_connections
            .store(count, std::sync::atomic::Ordering::Relaxed);
        let _ = metrics.update_tx.send(());
    }
}

pub(crate) async fn connected_voice_connection_count(manager: &songbird::Songbird) -> u32 {
    let calls: Vec<_> = manager.iter().map(|(_, call)| call).collect();
    let mut connected = 0_u32;
    for call in calls {
        if call.lock().await.current_channel().is_some() {
            connected = connected.saturating_add(1);
        }
    }
    connected
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
    fn routing_retries_only_connection_failures() {
        assert!(VoiceConnectOutcome::Failed("boom".to_string()).should_retry_routing());
        assert!(!VoiceConnectOutcome::AlreadyInChannel.should_retry_routing());
    }

    #[test]
    fn disconnect_success_only_for_real_disconnect() {
        assert!(VoiceDisconnectOutcome::Disconnected.success());
        assert!(!VoiceDisconnectOutcome::NotConnected.success());
        assert!(!VoiceDisconnectOutcome::VoiceSystemMissing.success());
        assert!(!VoiceDisconnectOutcome::Failed("boom".to_string()).success());
    }
}

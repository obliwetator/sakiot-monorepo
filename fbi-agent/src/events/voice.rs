use crate::cast::ToI64;
use crate::event_handler::Handler;
use serenity::{
    client::{Cache, Context},
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::{Pool, Postgres};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{error, warn};

pub(crate) mod coordinator;
mod session;
mod store;

pub use session::{VoiceConnectOutcome, VoiceDisconnectOutcome};
pub(super) use store::{
    EVT_RECORDING_PAUSE, EVT_RECORDING_RESUME, EVT_USER_RECORDING_PAUSE, EVT_USER_RECORDING_RESUME,
    insert_voice_event,
};

use coordinator::{GuildVoiceCoordinator, VoiceCoordinatorRegistry, VoiceCoordinatorRegistryKey};
use session::VoiceOperation;

const LOG_VOICE_STATE_CHANGES: bool = false;
const EMPTY_CHANNEL_LEAVE_DEBOUNCE: Duration = Duration::from_secs(3);
const VOICE_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60);
const ROUTE_RETRY_BACKOFF_SECONDS: [u64; 6] = [1, 2, 4, 8, 16, 30];

pub(crate) struct VoiceContextKey;

impl serenity::prelude::TypeMapKey for VoiceContextKey {
    type Value = Context;
}

pub async fn voice_server_update(
    _self: &Handler,
    _ctx: Context,
    _update: serenity::model::event::VoiceServerUpdateEvent,
) {
}

pub async fn connect_to_voice_channel(
    pool: Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    _user_id: u64,
) -> VoiceConnectOutcome {
    let registry = coordinator_registry(&ctx.data).await;
    let Some(registry) = registry else {
        return session::connect_to_voice_channel(pool, ctx, guild_id, channel_id).await;
    };
    let coordinator = registry.guild(guild_id);
    let _operation_guard = coordinator.operation.lock().await;
    let operation = build_operation(
        &pool,
        ctx,
        guild_id,
        &coordinator,
        "manual",
        population_snapshot(ctx, guild_id).unwrap_or_default(),
    )
    .await;
    session::connect_to_voice_channel_with_operation(pool, ctx, guild_id, channel_id, operation)
        .await
}

pub async fn disconnect_voice_channel(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> VoiceDisconnectOutcome {
    if let Some(registry) = coordinator_registry(data).await {
        let coordinator = registry.guild(guild_id);
        let _operation_guard = coordinator.operation.lock().await;
        return session::disconnect_voice_channel(data, pool, guild_id).await;
    }
    session::disconnect_voice_channel(data, pool, guild_id).await
}

pub(crate) async fn teardown_voice_session(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> session::VoiceTeardownReport {
    if let Some(registry) = coordinator_registry(data).await {
        let coordinator = registry.guild(guild_id);
        let _operation_guard = coordinator.operation.lock().await;
        return session::teardown_voice_session(data, pool, guild_id).await;
    }
    session::teardown_voice_session(data, pool, guild_id).await
}

pub(crate) async fn connected_voice_connection_count(manager: &songbird::Songbird) -> u32 {
    session::connected_voice_connection_count(manager).await
}

pub async fn voice_state_update(
    handler: &Handler,
    ctx: Context,
    old_state: Option<serenity::model::prelude::VoiceState>,
    new_state: serenity::model::prelude::VoiceState,
) {
    let is_own_bot = new_state.user_id == ctx.cache.current_user().id;
    let is_bot = is_own_bot
        || new_state
            .member
            .as_ref()
            .map(|member| member.user.bot)
            .or_else(|| {
                new_state.guild_id.and_then(|guild_id| {
                    ctx.cache
                        .guild(guild_id)
                        .and_then(|guild| guild.members.get(&new_state.user_id).map(|m| m.user.bot))
                })
            })
            .unwrap_or(false);

    track_active_voice_state_metrics(handler, &ctx, old_state.as_ref(), &new_state, is_bot).await;

    if !should_process_voice_transition(is_own_bot, is_bot) {
        return;
    }
    if should_skip_voice_state_for_lease(handler, new_state.guild_id).await {
        return;
    }

    store::record_voice_events(
        &handler.database,
        old_state.as_ref(),
        &new_state,
        LOG_VOICE_STATE_CHANGES,
    )
    .await;

    let Some(guild_id) = new_state.guild_id else {
        error!("No guild id in voice_state_update");
        return;
    };
    let transition = store::channel_transition(
        old_state.as_ref().and_then(|state| state.channel_id),
        new_state.channel_id,
    );
    if matches!(transition, store::ChannelTransition::Unchanged) {
        return;
    }

    let transition_at_ms = chrono::Utc::now().timestamp_millis();
    let afk_channel_id = guild_afk_channel(&ctx.cache, guild_id);
    let transition_delivered = crate::events::voice_receiver::user_voice_transition(
        &ctx,
        guild_id,
        new_state.user_id.get(),
        old_state.as_ref().and_then(|state| state.channel_id),
        new_state.channel_id,
        afk_channel_id,
        transition_at_ms,
    )
    .await;
    let entered_afk = new_state.channel_id.is_some() && new_state.channel_id == afk_channel_id;
    if !transition_delivered && (new_state.channel_id.is_none() || entered_afk) {
        let pending_cap_seconds = crate::database::logical_recordings::pending_cap_seconds(
            &handler.database,
            guild_id.to_i64(),
        )
        .await
        .unwrap_or(crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS);
        if let Err(err) = crate::database::logical_recordings::mark_pending_user_unavailable(
            &handler.database,
            crate::database::logical_recordings::PendingUserUnavailableRequest {
                guild_id: guild_id.to_i64(),
                user_id: new_state.user_id.to_i64(),
                at_ms: transition_at_ms,
                reason: if entered_afk { "afk" } else { "disconnect" },
                channel_id: new_state.channel_id.map(ToI64::to_i64),
                has_afk_channel: afk_channel_id.is_some(),
                pending_cap_seconds,
                owner_instance_id: &handler.runtime.config().instance_id,
            },
        )
        .await
        {
            warn!(
                guild_id = guild_id.get(),
                user_id = new_state.user_id.get(),
                "failed to persist pending-user transition without recorder actor: {}",
                err
            );
        }
    }

    let Some(registry) = coordinator_registry(&ctx.data).await else {
        warn!("voice coordinator registry missing during voice-state routing");
        return;
    };
    let coordinator = registry.guild(guild_id);
    coordinator.signal();
    route_latest(
        ctx,
        handler.database.clone(),
        handler.runtime.clone(),
        guild_id,
        coordinator,
        RouteOptions {
            trigger: "voice_state",
            allow_empty_debounce: true,
            schedule_retry: true,
        },
    )
    .await;
}

fn should_process_voice_transition(is_own_bot: bool, is_bot: bool) -> bool {
    !is_own_bot && !is_bot
}

pub(crate) fn spawn_reconciliation(
    custom: crate::Custom,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(VOICE_RECONCILIATION_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => reconcile_voice_sessions(&custom).await,
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }
    })
}

async fn reconcile_voice_sessions(custom: &crate::Custom) {
    // Serenity updates its cache before invoking event handlers. Reconcile
    // metrics from that independent snapshot so a skipped handler cannot keep
    // stale presence and start-time entries indefinitely.
    let mut current_voice_users = std::collections::HashSet::new();
    for guild_id in custom.cache.guilds() {
        if let Some(guild) = custom.cache.guild(guild_id) {
            for (user_id, voice_state) in &guild.voice_states {
                if voice_state.channel_id.is_some() {
                    current_voice_users.insert(crate::VoiceUserKey {
                        guild_id: guild_id.get(),
                        user_id: user_id.get(),
                    });
                }
            }
        }
    }
    {
        let data_read = custom.data.read().await;
        if let Some(metrics) = data_read.get::<crate::BotMetricsKey>() {
            metrics.prune_stale_voice_metrics(&current_voice_users);
        }
    }

    match crate::database::logical_recordings::recover_stale_sessions(
        &custom.pool,
        chrono::Utc::now().timestamp_millis(),
        crate::heartbeat::STALE_AFTER_SECONDS,
        None,
    )
    .await
    {
        Ok(report)
            if report.stale_pending_released > 0
                || report.stale_active_finalized > 0
                || report.overdue_finalized > 0 =>
        {
            warn!(
                stale_pending_released = report.stale_pending_released,
                stale_active_finalized = report.stale_active_finalized,
                overdue_finalized = report.overdue_finalized,
                "logical recording reconciliation repaired stale state"
            );
        }
        Ok(_) => {}
        Err(err) => warn!("logical recording reconciliation failed: {}", err),
    }

    let Some(registry) = coordinator_registry(&custom.data).await else {
        warn!("voice coordinator registry missing during reconciliation");
        return;
    };
    let ctx = {
        let data = custom.data.read().await;
        data.get::<VoiceContextKey>().cloned()
    };
    let Some(ctx) = ctx else {
        return;
    };
    for guild_id in custom.cache.guilds() {
        let coordinator = registry.guild(guild_id);
        coordinator.signal();
        route_latest(
            ctx.clone(),
            custom.pool.clone(),
            custom.runtime.clone(),
            guild_id,
            coordinator,
            RouteOptions {
                trigger: "reconciliation",
                allow_empty_debounce: true,
                schedule_retry: true,
            },
        )
        .await;
    }
}

async fn route_latest(
    ctx: Context,
    pool: Pool<Postgres>,
    runtime: Arc<crate::runtime::RuntimeState>,
    guild_id: GuildId,
    coordinator: Arc<GuildVoiceCoordinator>,
    options: RouteOptions,
) {
    let _operation_guard = coordinator.operation.lock().await;
    loop {
        let generation = coordinator.generation();
        let attempt = route_once_locked(
            &ctx,
            &pool,
            &runtime,
            guild_id,
            &coordinator,
            options.trigger,
            options.allow_empty_debounce,
        )
        .await;
        if attempt == RouteAttempt::Retry && options.schedule_retry {
            spawn_route_retry(
                ctx.clone(),
                pool.clone(),
                runtime.clone(),
                guild_id,
                coordinator.clone(),
            );
        }
        if coordinator.generation() == generation {
            break;
        }
    }
}

#[derive(Clone, Copy)]
struct RouteOptions {
    trigger: &'static str,
    allow_empty_debounce: bool,
    schedule_retry: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteAttempt {
    Stable,
    Retry,
}

async fn route_once_locked(
    ctx: &Context,
    pool: &Pool<Postgres>,
    runtime: &Arc<crate::runtime::RuntimeState>,
    guild_id: GuildId,
    coordinator: &Arc<GuildVoiceCoordinator>,
    trigger: &'static str,
    allow_empty_debounce: bool,
) -> RouteAttempt {
    let populations = match population_snapshot(ctx, guild_id) {
        Some(populations) => populations,
        None => {
            warn!(
                guild_id = guild_id.get(),
                "routing cache snapshot unavailable"
            );
            return RouteAttempt::Retry;
        }
    };
    let current_channel = current_bot_channel(ctx, guild_id).await;

    match choose_route(current_channel, &populations) {
        RoutingDecision::Stay => {
            coordinator.invalidate_empty_timer();
            RouteAttempt::Stable
        }
        RoutingDecision::Connect(channel_id) => {
            coordinator.invalidate_empty_timer();
            let operation = build_operation(
                pool,
                ctx,
                guild_id,
                coordinator,
                trigger,
                populations.clone(),
            )
            .await;
            let outcome = session::connect_to_voice_channel_with_operation(
                pool.clone(),
                ctx,
                guild_id,
                channel_id,
                operation,
            )
            .await;
            if outcome.should_retry_routing() {
                RouteAttempt::Retry
            } else {
                RouteAttempt::Stable
            }
        }
        RoutingDecision::EveryEligibleChannelEmpty => {
            if allow_empty_debounce {
                schedule_empty_recheck(
                    ctx.clone(),
                    pool.clone(),
                    runtime.clone(),
                    guild_id,
                    coordinator.clone(),
                );
                return RouteAttempt::Stable;
            }

            if let Some(channel_id) = current_channel {
                let cap = crate::database::logical_recordings::pending_cap_seconds(
                    pool,
                    guild_id.to_i64(),
                )
                .await
                .unwrap_or(crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS);
                crate::events::voice_receiver::begin_handoff(
                    ctx,
                    guild_id,
                    channel_id,
                    None,
                    guild_afk_channel(&ctx.cache, guild_id).is_some(),
                    cap,
                )
                .await;
            }
            let operation =
                build_operation(pool, ctx, guild_id, coordinator, trigger, populations).await;
            let report = session::teardown_voice_session_with_operation(
                &ctx.data,
                pool,
                guild_id,
                Some(&operation),
            )
            .await;
            if report.connected_after {
                RouteAttempt::Retry
            } else {
                RouteAttempt::Stable
            }
        }
    }
}

fn schedule_empty_recheck(
    ctx: Context,
    pool: Pool<Postgres>,
    runtime: Arc<crate::runtime::RuntimeState>,
    guild_id: GuildId,
    coordinator: Arc<GuildVoiceCoordinator>,
) {
    let token = coordinator.next_empty_timer();
    tokio::spawn(async move {
        tokio::time::sleep(EMPTY_CHANNEL_LEAVE_DEBOUNCE).await;
        if !coordinator.empty_timer_matches(token) {
            return;
        }
        coordinator.signal();
        route_latest(
            ctx,
            pool,
            runtime,
            guild_id,
            coordinator,
            RouteOptions {
                trigger: "empty_debounce",
                allow_empty_debounce: false,
                schedule_retry: true,
            },
        )
        .await;
    });
}

fn spawn_route_retry(
    ctx: Context,
    pool: Pool<Postgres>,
    runtime: Arc<crate::runtime::RuntimeState>,
    guild_id: GuildId,
    coordinator: Arc<GuildVoiceCoordinator>,
) {
    if !coordinator.begin_retry() {
        return;
    }
    tokio::spawn(async move {
        for delay_seconds in ROUTE_RETRY_BACKOFF_SECONDS {
            tokio::time::sleep(Duration::from_secs(delay_seconds)).await;
            coordinator.signal();
            let _operation_guard = coordinator.operation.lock().await;
            let attempt = route_once_locked(
                &ctx,
                &pool,
                &runtime,
                guild_id,
                &coordinator,
                "background_retry",
                true,
            )
            .await;
            if attempt == RouteAttempt::Stable {
                coordinator.finish_retry();
                return;
            }
        }
        coordinator.finish_retry();
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutingDecision {
    Stay,
    Connect(ChannelId),
    EveryEligibleChannelEmpty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
struct ChannelPopulation {
    #[serde(serialize_with = "serialize_channel_id")]
    channel_id: ChannelId,
    human_count: usize,
}

fn serialize_channel_id<S>(channel_id: &ChannelId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&channel_id.get().to_string())
}

fn choose_route(
    current_channel: Option<ChannelId>,
    populations: &[ChannelPopulation],
) -> RoutingDecision {
    let max_population = populations
        .iter()
        .map(|population| population.human_count)
        .max()
        .unwrap_or(0);
    if max_population == 0 {
        return RoutingDecision::EveryEligibleChannelEmpty;
    }

    let current_population = current_channel
        .and_then(|channel_id| {
            populations
                .iter()
                .find(|population| population.channel_id == channel_id)
                .map(|population| population.human_count)
        })
        .unwrap_or(0);
    if current_channel.is_some() && current_population == max_population {
        return RoutingDecision::Stay;
    }

    populations
        .iter()
        .filter(|population| population.human_count == max_population)
        .min_by_key(|population| population.channel_id.get())
        .map(|population| RoutingDecision::Connect(population.channel_id))
        .unwrap_or(RoutingDecision::EveryEligibleChannelEmpty)
}

fn population_snapshot(ctx: &Context, guild_id: GuildId) -> Option<Vec<ChannelPopulation>> {
    let guild = ctx.cache.guild(guild_id)?;
    let afk_channel = guild
        .afk_metadata
        .as_ref()
        .map(|metadata| metadata.afk_channel_id);
    let mut populations: Vec<ChannelPopulation> = guild
        .channels
        .values()
        .filter(|channel| {
            matches!(
                channel.kind,
                serenity::model::channel::ChannelType::Voice
                    | serenity::model::channel::ChannelType::Stage
            ) && Some(channel.id) != afk_channel
        })
        .map(|channel| ChannelPopulation {
            channel_id: channel.id,
            human_count: 0,
        })
        .collect();
    populations.sort_by_key(|population| population.channel_id.get());

    for (user_id, voice_state) in &guild.voice_states {
        let Some(channel_id) = voice_state.channel_id else {
            continue;
        };
        let is_bot = voice_state
            .member
            .as_ref()
            .map(|member| member.user.bot)
            .or_else(|| guild.members.get(user_id).map(|member| member.user.bot));
        if is_bot != Some(false) {
            continue;
        }
        if let Some(population) = populations
            .iter_mut()
            .find(|population| population.channel_id == channel_id)
        {
            population.human_count = population.human_count.saturating_add(1);
        }
    }
    Some(populations)
}

async fn current_bot_channel(ctx: &Context, guild_id: GuildId) -> Option<ChannelId> {
    if let Some(guild) = ctx.cache.guild(guild_id) {
        return guild
            .voice_states
            .get(&ctx.cache.current_user().id)
            .and_then(|state| state.channel_id);
    }
    let manager = songbird::get(ctx).await?;
    let call = manager.get(guild_id)?;
    let channel = call.lock().await.current_channel()?;
    Some(ChannelId::new(channel.0.get()))
}

fn guild_afk_channel(cache: &Cache, guild_id: GuildId) -> Option<ChannelId> {
    cache.guild(guild_id).and_then(|guild| {
        guild
            .afk_metadata
            .as_ref()
            .map(|metadata| metadata.afk_channel_id)
    })
}

async fn build_operation(
    pool: &Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    coordinator: &GuildVoiceCoordinator,
    trigger: &str,
    populations: Vec<ChannelPopulation>,
) -> VoiceOperation {
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    let runtime = crate::runtime::state_from_ctx(ctx).await;
    let instance_id = runtime
        .as_ref()
        .map(|runtime| runtime.config().instance_id.as_str())
        .unwrap_or("unknown");
    let pending_cap_seconds =
        crate::database::logical_recordings::pending_cap_seconds(pool, guild_id.to_i64())
            .await
            .unwrap_or_else(|err| {
                warn!(
                    guild_id = guild_id.get(),
                    "failed to load pending recording cap: {}", err
                );
                crate::database::logical_recordings::DEFAULT_PENDING_CAP_SECONDS
            });
    VoiceOperation {
        operation_id: coordinator.operation_id(instance_id, guild_id, started_at_ms),
        trigger: trigger.to_string(),
        started_at_ms,
        population_snapshot: serde_json::to_value(populations)
            .unwrap_or_else(|_| serde_json::json!([])),
        has_afk_channel: guild_afk_channel(&ctx.cache, guild_id).is_some(),
        pending_cap_seconds,
    }
}

async fn coordinator_registry(
    data: &Arc<RwLock<TypeMap>>,
) -> Option<Arc<VoiceCoordinatorRegistry>> {
    data.read()
        .await
        .get::<VoiceCoordinatorRegistryKey>()
        .cloned()
}

async fn track_active_voice_state_metrics(
    handler: &Handler,
    ctx: &Context,
    old_state: Option<&serenity::model::prelude::VoiceState>,
    new_state: &serenity::model::prelude::VoiceState,
    is_bot: bool,
) {
    if handler.runtime.is_draining() {
        return;
    }

    let data_read = ctx.data.read().await;
    let Some(metrics) = data_read.get::<crate::BotMetricsKey>() else {
        return;
    };

    metrics.record_voice_state_update();
    let _ = metrics.voice_update_tx.send(());

    let user_id = new_state.user_id.get();
    if let Some(guild_id) = new_state.guild_id {
        metrics.track_voice_presence(
            guild_id.get(),
            user_id,
            new_state
                .channel_id
                .map(|channel_id| crate::VoiceUserPresence {
                    channel_id: channel_id.get(),
                    is_bot,
                    server_mute: new_state.mute,
                    server_deaf: new_state.deaf,
                    self_mute: new_state.self_mute,
                    self_deaf: new_state.self_deaf,
                    suppress: new_state.suppress,
                    streaming: new_state.self_stream.unwrap_or(false),
                    video: new_state.self_video,
                }),
        );
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_i64())
        .unwrap_or_else(|err| {
            error!("System clock before UNIX_EPOCH: {}", err);
            0
        });

    if let Some(new_ch) = new_state.channel_id {
        if let Some(old) = old_state {
            if old.channel_id != Some(new_ch) {
                metrics.user_start_times.insert(user_id, now);
            }
        } else {
            metrics.user_start_times.insert(user_id, now);
        }
    } else {
        metrics.user_start_times.remove(&user_id);
    }
}

async fn should_skip_voice_state_for_lease(handler: &Handler, guild_id: Option<GuildId>) -> bool {
    let Some(guild_id) = guild_id else {
        return handler.runtime.is_draining();
    };

    match crate::deployment::active_lease_owner(&handler.database, guild_id).await {
        Ok(Some(owner)) => owner != handler.runtime.config().instance_id,
        Ok(None) => handler.runtime.is_draining(),
        Err(err) => {
            warn!(
                guild_id = guild_id.get(),
                "failed to inspect voice lease before voice-state routing: {}", err
            );
            handler.runtime.is_draining()
        }
    }
}

#[cfg(test)]
mod tests {
    use serenity::model::id::ChannelId;

    use super::{
        ChannelPopulation, RoutingDecision, choose_route, should_process_voice_transition,
    };

    fn population(channel_id: u64, human_count: usize) -> ChannelPopulation {
        ChannelPopulation {
            channel_id: ChannelId::new(channel_id),
            human_count,
        }
    }

    #[test]
    fn missing_member_does_not_block_human_transition() {
        assert!(should_process_voice_transition(false, false));
    }

    #[test]
    fn own_bot_event_is_ignored_by_user_id() {
        assert!(!should_process_voice_transition(true, true));
    }

    #[test]
    fn strict_largest_channel_wins() {
        assert_eq!(
            choose_route(
                Some(ChannelId::new(10)),
                &[population(10, 2), population(20, 3)]
            ),
            RoutingDecision::Connect(ChannelId::new(20))
        );
    }

    #[test]
    fn equal_population_keeps_current_channel() {
        assert_eq!(
            choose_route(
                Some(ChannelId::new(20)),
                &[population(10, 3), population(20, 3)]
            ),
            RoutingDecision::Stay
        );
    }

    #[test]
    fn disconnected_tie_uses_stable_channel_id() {
        assert_eq!(
            choose_route(None, &[population(20, 3), population(10, 3)]),
            RoutingDecision::Connect(ChannelId::new(10))
        );
    }

    #[test]
    fn every_empty_channel_uses_debounced_path() {
        assert_eq!(
            choose_route(
                Some(ChannelId::new(10)),
                &[population(10, 0), population(20, 0)]
            ),
            RoutingDecision::EveryEligibleChannelEmpty
        );
    }

    #[test]
    fn leave_recalculation_moves_to_remaining_largest_channel() {
        assert_eq!(
            choose_route(
                Some(ChannelId::new(10)),
                &[population(10, 0), population(20, 2)]
            ),
            RoutingDecision::Connect(ChannelId::new(20))
        );
    }
}

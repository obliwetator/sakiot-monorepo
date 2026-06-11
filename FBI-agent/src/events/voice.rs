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
use tracing::{error, info, warn};

mod session;
mod store;

pub use session::{VoiceConnectOutcome, VoiceDisconnectOutcome};
pub(super) use store::{
    EVT_RECORDING_PAUSE, EVT_RECORDING_RESUME, EVT_USER_RECORDING_PAUSE, EVT_USER_RECORDING_RESUME,
    insert_voice_event,
};

const LOG_VOICE_STATE_CHANGES: bool = false;
const EMPTY_CHANNEL_LEAVE_DEBOUNCE: Duration = Duration::from_secs(3);
const VOICE_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60);

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
    session::connect_to_voice_channel(pool, ctx, guild_id, channel_id).await
}

pub async fn disconnect_voice_channel(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> VoiceDisconnectOutcome {
    session::disconnect_voice_channel(data, pool, guild_id).await
}

pub(crate) async fn teardown_voice_session(
    data: &Arc<RwLock<TypeMap>>,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> session::VoiceTeardownReport {
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

    let guild_id = match new_state.guild_id {
        Some(guild_id) => guild_id,
        None => {
            error!("No guild id in voice_state_update");
            return;
        }
    };

    let transition = store::channel_transition(
        old_state.as_ref().and_then(|state| state.channel_id),
        new_state.channel_id,
    );
    if matches!(transition, store::ChannelTransition::Unchanged) {
        return;
    }

    if let Some(old_channel_id) = old_channel_to_recheck(transition) {
        schedule_leave_if_still_empty(
            ctx.clone(),
            handler.database.clone(),
            guild_id,
            old_channel_id,
        );
    }

    if matches!(transition, store::ChannelTransition::Left(_)) {
        return;
    }

    let (highest_channel_id, highest_channel_len) =
        match get_channel_with_most_members(handler, &ctx, &new_state).await {
            Some(value) => value,
            None => {
                warn!(
                    guild_id = guild_id.get(),
                    "Skipping voice join because channel membership could not be inspected"
                );
                return;
            }
        };

    if highest_channel_len > 0 {
        connect_to_voice_channel(
            handler.database.clone(),
            &ctx,
            guild_id,
            highest_channel_id,
            new_state.user_id.get(),
        )
        .await;
    }
}

fn should_process_voice_transition(is_own_bot: bool, is_bot: bool) -> bool {
    !is_own_bot && !is_bot
}

fn old_channel_to_recheck(transition: store::ChannelTransition) -> Option<ChannelId> {
    match transition {
        store::ChannelTransition::Left(channel_id) => Some(channel_id),
        store::ChannelTransition::Switched { from, .. } => Some(from),
        store::ChannelTransition::Joined(_) | store::ChannelTransition::Unchanged => None,
    }
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
    let manager = {
        let data = custom.data.read().await;
        data.get::<songbird::SongbirdKey>().cloned()
    };
    let Some(manager) = manager else {
        warn!("Songbird manager missing during voice reconciliation");
        return;
    };

    let calls: Vec<_> = manager.iter().collect();
    for (songbird_guild_id, call) in calls {
        let guild_id = GuildId::new(songbird_guild_id.0.get());
        let current_channel = call
            .lock()
            .await
            .current_channel()
            .map(|channel| ChannelId::new(channel.0.get()));
        let human_count = current_channel
            .and_then(|channel_id| cached_human_member_count(&custom.cache, guild_id, channel_id));

        match reconciliation_decision(current_channel.is_some(), human_count) {
            ReconciliationDecision::RetainOccupied | ReconciliationDecision::RetainUnknown => {}
            ReconciliationDecision::TeardownDisconnected => {
                info!(
                    guild_id = guild_id.get(),
                    "reconciling disconnected Songbird call"
                );
                teardown_voice_session(&custom.data, &custom.pool, guild_id).await;
            }
            ReconciliationDecision::TeardownEmpty => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = current_channel.map(ChannelId::get),
                    "reconciling empty voice channel"
                );
                teardown_voice_session(&custom.data, &custom.pool, guild_id).await;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReconciliationDecision {
    RetainOccupied,
    RetainUnknown,
    TeardownDisconnected,
    TeardownEmpty,
}

fn reconciliation_decision(connected: bool, human_count: Option<usize>) -> ReconciliationDecision {
    if !connected {
        ReconciliationDecision::TeardownDisconnected
    } else {
        match human_count {
            Some(0) => ReconciliationDecision::TeardownEmpty,
            Some(_) => ReconciliationDecision::RetainOccupied,
            None => ReconciliationDecision::RetainUnknown,
        }
    }
}

fn cached_human_member_count(
    cache: &Cache,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Option<usize> {
    let guild = cache.guild(guild_id)?;
    let bot_id = cache.current_user().id;
    let mut humans = 0;

    for (user_id, voice_state) in &guild.voice_states {
        if voice_state.channel_id != Some(channel_id) || *user_id == bot_id {
            continue;
        }
        let member = guild.members.get(user_id)?;
        if !member.user.bot {
            humans += 1;
        }
    }

    Some(humans)
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

async fn human_member_count(ctx: &Context, channel_id: ChannelId) -> Result<usize, String> {
    let current_channel = channel_id
        .to_channel(ctx)
        .await
        .map_err(|err| format!("could not resolve channel: {}", err))?;

    let guild_channel = current_channel
        .guild()
        .ok_or_else(|| "not a guild channel".to_string())?;

    let mut members = guild_channel
        .members(ctx)
        .map_err(|err| format!("could not get channel members: {}", err))?;

    members.retain(|member| !member.user.bot);
    Ok(members.len())
}

fn schedule_leave_if_still_empty(
    ctx: Context,
    pool: Pool<Postgres>,
    guild_id: GuildId,
    channel_id: ChannelId,
) {
    tokio::spawn(async move {
        info!(
            guild_id = guild_id.get(),
            channel_id = channel_id.get(),
            "empty channel leave scheduled"
        );

        tokio::time::sleep(EMPTY_CHANNEL_LEAVE_DEBOUNCE).await;

        let Some(manager) = songbird::get(&ctx).await else {
            error!("Songbird manager missing while rechecking empty voice channel");
            return;
        };
        let manager = manager.clone();

        let Some(call) = manager.get(guild_id) else {
            info!(
                guild_id = guild_id.get(),
                channel_id = channel_id.get(),
                "empty channel leave cancelled: bot moved/disconnected"
            );
            return;
        };

        let current_channel = call
            .lock()
            .await
            .current_channel()
            .map(|channel| channel.0.get());
        if current_channel != Some(channel_id.get()) {
            info!(
                guild_id = guild_id.get(),
                channel_id = channel_id.get(),
                current_channel = current_channel,
                "empty channel leave cancelled: bot moved/disconnected"
            );
            return;
        }

        match human_member_count(&ctx, channel_id).await {
            Ok(0) => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    "empty channel leave confirmed"
                );
                disconnect_voice_channel(&ctx.data, &pool, guild_id).await;
            }
            Ok(count) => {
                info!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    humans = count,
                    "empty channel leave cancelled: humans returned"
                );
            }
            Err(err) => {
                warn!(
                    guild_id = guild_id.get(),
                    channel_id = channel_id.get(),
                    "empty channel leave cancelled: could not inspect channel: {}",
                    err
                );
            }
        }
    });
}

async fn get_channel_with_most_members(
    handler: &Handler,
    ctx: &Context,
    new_state: &serenity::model::prelude::VoiceState,
) -> Option<(ChannelId, usize)> {
    let guild_id = match new_state.guild_id {
        Some(id) => id,
        None => return None,
    };

    // Extract only the single value we need, then drop the read guard immediately.
    // Holding the guard across the channel-iteration loop (which calls into the cache)
    // would block any concurrent writer (e.g. cache_ready) for the entire duration.
    let afk_channel_id_option: Option<u64> = {
        let lock_guard = handler.afk_channels.read().await;
        lock_guard.get(&guild_id.get()).copied().unwrap_or(None)
    };

    // Clone the channels out of the cache so we don't hold a DashMap guard
    // while doing further cache lookups inside the loop (guild_channel.members).
    let channels: Vec<_> = match ctx.cache.guild(guild_id) {
        Some(guild) => guild.channels.values().cloned().collect(),
        None => {
            error!(
                "Guild {} missing from cache while choosing voice channel",
                guild_id
            );
            return None;
        }
    };

    let mut highest_channel_id: ChannelId = ChannelId::new(1);
    let mut highest_channel_len: usize = 0;
    for guild_channel in &channels {
        let channel_id = guild_channel.id;
        if let Some(afk_channel_id) = afk_channel_id_option {
            // Ignore channels that are meant for afk
            if afk_channel_id == channel_id.get() {
                continue;
            }
        }
        if let serenity::model::prelude::ChannelType::Voice = guild_channel.kind {
            let count = match human_member_count(ctx, channel_id).await {
                Ok(count) => count,
                Err(err) => {
                    warn!(
                        channel_id = channel_id.get(),
                        "Could not inspect voice channel members: {}", err
                    );
                    return None;
                }
            };

            if count > highest_channel_len {
                highest_channel_len = count;
                highest_channel_id = guild_channel.id;
            }
        }
    }
    Some((highest_channel_id, highest_channel_len))
}

#[cfg(test)]
mod tests {
    use serenity::model::id::ChannelId;

    use super::{
        ReconciliationDecision, old_channel_to_recheck, reconciliation_decision,
        should_process_voice_transition,
    };
    use crate::events::voice::store::ChannelTransition;

    #[test]
    fn missing_member_does_not_block_human_transition() {
        assert!(should_process_voice_transition(false, false));
    }

    #[test]
    fn own_bot_event_is_ignored_by_user_id() {
        assert!(!should_process_voice_transition(true, true));
    }

    #[test]
    fn leave_and_switch_recheck_old_channel() {
        let old = ChannelId::new(10);
        let new = ChannelId::new(20);
        assert_eq!(
            old_channel_to_recheck(ChannelTransition::Left(old)),
            Some(old)
        );
        assert_eq!(
            old_channel_to_recheck(ChannelTransition::Switched { from: old, to: new }),
            Some(old)
        );
    }

    #[test]
    fn unchanged_transition_does_not_recheck_channel() {
        assert_eq!(old_channel_to_recheck(ChannelTransition::Unchanged), None);
    }

    #[test]
    fn reconciliation_removes_empty_channel() {
        assert_eq!(
            reconciliation_decision(true, Some(0)),
            ReconciliationDecision::TeardownEmpty
        );
    }

    #[test]
    fn reconciliation_retains_occupied_channel() {
        assert_eq!(
            reconciliation_decision(true, Some(1)),
            ReconciliationDecision::RetainOccupied
        );
    }

    #[test]
    fn reconciliation_retains_unknown_cache_state() {
        assert_eq!(
            reconciliation_decision(true, None),
            ReconciliationDecision::RetainUnknown
        );
    }

    #[test]
    fn reconciliation_releases_disconnected_entry() {
        assert_eq!(
            reconciliation_decision(false, None),
            ReconciliationDecision::TeardownDisconnected
        );
    }
}

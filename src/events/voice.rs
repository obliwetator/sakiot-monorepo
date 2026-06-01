use crate::event_handler::Handler;
use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::{Pool, Postgres};
use std::{sync::Arc, time::Duration};
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

pub async fn voice_state_update(
    _self: &Handler,
    ctx: Context,
    old_state: Option<serenity::model::prelude::VoiceState>,
    new_state: serenity::model::prelude::VoiceState,
) {
    let is_bot = new_state
        .member
        .as_ref()
        .map(|m| m.user.bot)
        .unwrap_or(false);

    track_active_voice_state_metrics(_self, &ctx, old_state.as_ref(), &new_state, is_bot).await;

    if should_skip_voice_state_for_lease(_self, new_state.guild_id).await {
        return;
    }

    // Persist voice events for non-bot users (timeline overlay on recordings).
    if !is_bot {
        store::record_voice_events(
            &_self.database,
            old_state.as_ref(),
            &new_state,
            LOG_VOICE_STATE_CHANGES,
        )
        .await;
    }

    if let Some(member) = &new_state.member {
        let guild_id = match new_state.guild_id {
            Some(ok) => ok,
            None => {
                error!("No guild id in voice_state_update");
                return;
            }
        };
        if member.user.bot {
            // Ignore bots
            return;
        }

        if let Some(channel_id) = empty_channel_candidate(&new_state, &ctx, &old_state).await {
            schedule_leave_if_still_empty(
                ctx.clone(),
                _self.database.clone(),
                guild_id,
                channel_id,
            );
            return;
        }

        if let Some(old) = old_state
            && let Some(new_channel_id) = new_state.channel_id
            && let Some(old_channel_id) = old.channel_id
        {
            // We can check for various things that happened after the user has connected
            // We don't care about any events at the moment
            if new_channel_id == old_channel_id {
                // An action happened that was NOT switching channels.
                return;
            } else {
                // user switched channels
            }
        }

        let (highest_channel_id, highest_channel_len) =
            match get_channel_with_most_members(_self, &ctx, &new_state).await {
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
            let user_id = new_state.user_id;
            connect_to_voice_channel(
                _self.database.clone(),
                &ctx,
                guild_id,
                highest_channel_id,
                user_id.get(),
            )
            .await;
        }
    } else {
        error!("No member in new_state");
    }
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
        .map(|d| d.as_secs() as i64)
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

// If no humans remain, schedule a delayed re-check before leaving.
async fn empty_channel_candidate(
    new_state: &serenity::model::prelude::VoiceState,
    ctx: &Context,
    old_state: &Option<serenity::model::prelude::VoiceState>,
) -> Option<ChannelId> {
    if new_state.channel_id.is_none() {
        // Someone left the channel

        // get the channel id that user was in before disconnecting
        if let Some(channel_id) = old_state.as_ref().and_then(|s| s.channel_id) {
            match human_member_count(ctx, channel_id).await {
                Ok(0) => return Some(channel_id),
                Ok(_) => return None,
                Err(err) => {
                    warn!(
                        channel_id = channel_id.get(),
                        "Could not inspect channel before scheduling leave: {}", err
                    );
                    return None;
                }
            }
        }
    }
    None
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

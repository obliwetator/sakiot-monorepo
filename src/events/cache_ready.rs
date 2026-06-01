use serenity::client::Context;
use serenity::model::guild::Guild;
use serenity::model::id::GuildId;
use tracing::error;

use crate::database;
use crate::event_handler::Handler;

pub async fn cache_ready(handler: &Handler, ctx: Context, guilds: Vec<GuildId>) {
    let guild_cached: Vec<Guild> = guilds
        .iter()
        .filter_map(|guild| {
            guild
                .to_guild_cached(&ctx)
                .map(|g| g.to_owned())
                .or_else(|| {
                    error!("Guild {} missing from cache during cache_ready", guild);
                    None
                })
        })
        .collect();

    {
        // Acquire the write lock once for the entire loop instead of once per
        // iteration. Repeatedly dropping and re-acquiring the write lock lets
        // readers (e.g. get_channel_with_most_members) slip in between iterations
        // and causes unnecessary contention.
        let mut guard = handler.afk_channels.write().await;
        for ele in &guild_cached {
            let afk = ele.afk_metadata.as_ref().map(|m| m.afk_channel_id.get());
            guard.insert(ele.id.get(), afk);
        }
    }

    seed_voice_presence_metrics(handler, &ctx, &guild_cached).await;
    let _ = database::update_info(handler, &ctx, &guilds).await;
    database::user_names::seed_from_guilds(&handler.database, &guild_cached).await;
}

async fn seed_voice_presence_metrics(handler: &Handler, ctx: &Context, guilds: &[Guild]) {
    if handler.runtime.is_draining() {
        return;
    }

    let data = ctx.data.read().await;
    let Some(metrics) = data.get::<crate::BotMetricsKey>() else {
        return;
    };

    metrics.clear_voice_presence();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for guild in guilds {
        for (user_id, voice_state) in &guild.voice_states {
            let Some(channel_id) = voice_state.channel_id else {
                continue;
            };
            let is_bot = guild
                .members
                .get(user_id)
                .map(|member| member.user.bot)
                .unwrap_or(false);

            metrics.track_voice_presence(
                guild.id.get(),
                user_id.get(),
                Some(crate::VoiceUserPresence {
                    channel_id: channel_id.get(),
                    is_bot,
                    server_mute: voice_state.mute,
                    server_deaf: voice_state.deaf,
                    self_mute: voice_state.self_mute,
                    self_deaf: voice_state.self_deaf,
                    suppress: voice_state.suppress,
                    streaming: voice_state.self_stream.unwrap_or(false),
                    video: voice_state.self_video,
                }),
            );
            metrics.user_start_times.insert(user_id.get(), now);
        }
    }

    let _ = metrics.voice_update_tx.send(());
}

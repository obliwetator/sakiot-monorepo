use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::id::{ChannelId, UserId};
use serenity::model::prelude::CommandOptionType;

use crate::events::voice_receiver::clips_file_path;
use serenity::model::prelude::GuildId;
use songbird::Songbird;
use sqlx::{Pool, Postgres};

pub async fn play_clip(
    pool: &Pool<Postgres>,
    manager: &std::sync::Arc<Songbird>,
    guild_id: GuildId,
    clip_id: &str,
    user_id: i64,
) -> Result<String, String> {
    let row = sqlx::query!(
        "SELECT saved_file_name, name FROM clips WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
        guild_id.get() as i64,
        clip_id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    let (saved_file_name, actual_name) = if let Some(record) = row {
        let actual = record.name.unwrap_or_else(|| clip_id.to_string());
        let saved = record
            .saved_file_name
            .unwrap_or_else(|| format!("{}.ogg", clip_id));
        (saved, actual)
    } else {
        return Err(format!("Clip with ID '{}' not found in database.", clip_id));
    };

    let result = songbird::input::File::new(clips_file_path().join(saved_file_name));
    let input = songbird::input::Input::from(result);

    let handler = match manager.get(guild_id) {
        Some(h) => h,
        None => return Err("I am not currently in a voice channel.".to_string()),
    };

    let handler_lock = handler.lock().await.enqueue(input.into()).await;
    let _ = handler_lock.set_volume(0.5);

    if let Err(e) = sqlx::query!(
        "INSERT INTO jam_invocations (user_id, guild_id, clip_id) VALUES ($1, $2, $3)",
        user_id,
        guild_id.get() as i64,
        clip_id
    )
    .execute(pool)
    .await
    {
        tracing::warn!(
            "Failed to record jam invocation (user={}, guild={}, clip={}): {}",
            user_id,
            guild_id,
            clip_id,
            e
        );
    }

    Ok(format!("Now jamming: {}", actual_name))
}

pub fn register_jam() -> CreateCommand {
    CreateCommand::new("jam")
        .description("Play a clip in the current voice channel")
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "clip", "The clip to play")
                .required(true)
                .set_autocomplete(true),
        )
}

pub async fn queue(manager: &std::sync::Arc<Songbird>, guild_id: GuildId) -> String {
    if let Some(handler) = manager.get(guild_id) {
        let handler_lock = handler.lock().await;
        let queue = handler_lock.queue();
        let queue_len = queue.len();
        format!("There are {} tracks in the queue.", queue_len)
    } else {
        "Not in a voice channel.".to_string()
    }
}

pub async fn skip(manager: &std::sync::Arc<Songbird>, guild_id: GuildId) -> String {
    if let Some(handler) = manager.get(guild_id) {
        let handler_lock = handler.lock().await;
        let queue = handler_lock.queue();
        let _ = queue.skip();
        "Skipped the current track.".to_string()
    } else {
        "Not in a voice channel.".to_string()
    }
}

pub async fn stop(manager: &std::sync::Arc<Songbird>, guild_id: GuildId) -> String {
    if let Some(handler) = manager.get(guild_id) {
        let handler_lock = handler.lock().await;
        let queue = handler_lock.queue();
        queue.stop();
        "Stopped playback and cleared the queue.".to_string()
    } else {
        "Not in a voice channel.".to_string()
    }
}

pub async fn join(
    pool: &Pool<Postgres>,
    ctx: &Context,
    guild_id: GuildId,
    user_id: UserId,
) -> String {
    let channel_id = match user_voice_channel(ctx, guild_id, user_id) {
        Some(id) => id,
        None => return "You must be in a voice channel to use /join.".to_string(),
    };

    let outcome = crate::events::voice::connect_to_voice_channel(
        pool.clone(),
        ctx,
        guild_id,
        channel_id,
        user_id.get(),
    )
    .await;

    outcome.user_message()
}

fn user_voice_channel(ctx: &Context, guild_id: GuildId, user_id: UserId) -> Option<ChannelId> {
    let guild = ctx.cache.guild(guild_id)?;
    let voice_state = guild.voice_states.get(&user_id)?;
    voice_state.channel_id
}

pub fn register_queue() -> CreateCommand {
    CreateCommand::new("queue").description("List how many tracks are in the queue")
}

pub fn register_skip() -> CreateCommand {
    CreateCommand::new("skip").description("Skip the currently playing track")
}

pub fn register_stop() -> CreateCommand {
    CreateCommand::new("stop").description("Stop playback and clear the queue")
}

pub fn register_join() -> CreateCommand {
    CreateCommand::new("join").description("Join your current voice channel")
}

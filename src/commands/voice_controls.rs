use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::id::{ChannelId, UserId};
use serenity::model::prelude::CommandOptionType;

use crate::database::DbError;
use crate::events::voice_receiver::clips_file_path;
use serenity::model::prelude::GuildId;
use songbird::Songbird;
use sqlx::{Pool, Postgres};

#[derive(Debug, thiserror::Error)]
pub enum PlayClipError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("{0}")]
    User(String),
}

impl PlayClipError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Db(_) => "Database error. Try again later.".to_string(),
            Self::User(message) => message.clone(),
        }
    }
}

pub async fn play_clip(
    pool: &Pool<Postgres>,
    manager: &std::sync::Arc<Songbird>,
    guild_id: GuildId,
    clip_id: &str,
    user_id: i64,
) -> Result<String, PlayClipError> {
    let clip = crate::database::clips::playable_clip(pool, guild_id.get() as i64, clip_id)
        .await
        .map_err(PlayClipError::Db)?
        .ok_or_else(|| PlayClipError::User(format!("Clip with ID '{}' not found.", clip_id)))?;

    let result = songbird::input::File::new(clips_file_path().join(clip.saved_file_name));
    let input = songbird::input::Input::from(result);

    let handler = match manager.get(guild_id) {
        Some(h) => h,
        None => {
            return Err(PlayClipError::User(
                "I am not currently in a voice channel.".to_string(),
            ));
        }
    };

    crate::database::clips::record_jam_invocation(pool, user_id, guild_id.get() as i64, clip_id)
        .await
        .map_err(PlayClipError::Db)?;

    let handler_lock = handler.lock().await.enqueue(input.into()).await;
    let _ = handler_lock.set_volume(0.5);

    Ok(format!("Now jamming: {}", clip.display_name))
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

use crate::cast::ToI64;
use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::{Cache, Context};
use serenity::model::id::{ChannelId, UserId};
use serenity::model::prelude::{ChannelType, CommandOptionType, Permissions};

use crate::cooldown::{CheckResult, JamCooldown};
use crate::database::DbError;
use serenity::model::prelude::GuildId;
use songbird::Songbird;
use sqlx::{Pool, Postgres};

#[derive(Debug, thiserror::Error)]
pub enum PlayClipError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Media(#[from] crate::media_archive::MediaArchiveError),
    /// This instance holds no voice connection for the guild. Distinct from `User` so
    /// the gRPC path can answer `NotPresent` rather than a generic failure.
    #[error("I am not currently in a voice channel.")]
    NotInVoice,
    /// The clip was ready to play, but the user played one too recently.
    #[error("On cooldown — {remaining_secs}s remaining.")]
    Cooldown { remaining_secs: u32 },
    #[error("{0}")]
    User(String),
}

impl PlayClipError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Db(_) => "Database error. Try again later.".to_string(),
            Self::Media(_) => "Clip media is unavailable. Try again later.".to_string(),
            Self::NotInVoice | Self::Cooldown { .. } => self.to_string(),
            Self::User(message) => message.clone(),
        }
    }
}

/// The handles playing a clip reaches into. The gRPC, slash-command, and replay
/// callers each hold them separately.
pub struct ClipPlayer<'a> {
    pub pool: &'a Pool<Postgres>,
    pub media_archive: &'a crate::media_archive::MediaArchive,
    pub manager: &'a std::sync::Arc<Songbird>,
    pub cache: &'a Cache,
    pub cooldown: &'a JamCooldown,
}

/// Queues a clip in the guild's voice call for the user.
///
/// The user's jam cooldown is spent last, once this instance is in the call, the
/// clip is theirs to play, and its media is local, so a failed attempt never costs
/// them their turn.
pub async fn play_clip(
    player: &ClipPlayer<'_>,
    guild_id: GuildId,
    clip_id: &str,
    user_id: i64,
) -> Result<String, PlayClipError> {
    let ClipPlayer {
        pool,
        media_archive,
        manager,
        cache,
        cooldown,
    } = *player;
    // Answer this from our own songbird, not from the guild cache: the bot user id
    // is shared with any draining instance, so seeing "the bot" in a voice channel
    // says nothing about whether *this* process holds that connection. Songbird
    // also keeps a `Call` after a disconnect, so the call must still have a channel.
    let Some(handler) = manager.get(guild_id) else {
        return Err(PlayClipError::NotInVoice);
    };
    if handler.lock().await.current_channel().is_none() {
        return Err(PlayClipError::NotInVoice);
    }

    let requester = u64::try_from(user_id)
        .ok()
        .filter(|user_id| *user_id != 0)
        .ok_or_else(|| PlayClipError::User("Clip is not available to you.".to_string()))?;
    let visible_channel_ids = visible_voice_channel_ids(cache, guild_id, UserId::new(requester));
    let clip = crate::database::clips::playable_clip(
        pool,
        guild_id.to_i64(),
        clip_id,
        &visible_channel_ids,
    )
    .await
    .map_err(PlayClipError::Db)?
    .ok_or_else(|| PlayClipError::User("Clip is not available to you.".to_string()))?;

    let clip_path = media_archive
        .ensure_clip_local(pool, &clip.clip_id, &clip.saved_file_name)
        .await?;

    match cooldown
        .check_and_record(pool, guild_id.to_i64(), user_id)
        .await
        .map_err(PlayClipError::Db)?
    {
        CheckResult::Allowed => {}
        CheckResult::OnCooldown { remaining_secs } => {
            return Err(PlayClipError::Cooldown { remaining_secs });
        }
    }

    let input = songbird::input::Input::from(songbird::input::File::new(clip_path));
    crate::database::clips::record_jam_invocation(pool, user_id, guild_id.to_i64(), &clip.clip_id)
        .await
        .map_err(PlayClipError::Db)?;

    let track = handler.lock().await.enqueue(input.into()).await;
    let _ = track.set_volume(0.5);

    Ok(format!("Now jamming: {}", clip.display_name))
}

pub(crate) fn visible_voice_channel_ids(
    cache: &Cache,
    guild_id: GuildId,
    user_id: UserId,
) -> Vec<i64> {
    let Some(guild) = cache.guild(guild_id) else {
        return Vec::new();
    };
    let Some(member) = guild.members.get(&user_id) else {
        return Vec::new();
    };

    guild
        .channels
        .values()
        .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage))
        .filter(|channel| {
            guild
                .user_permissions_in(channel, member)
                .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT)
        })
        .map(|channel| channel.id.to_i64())
        .collect()
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

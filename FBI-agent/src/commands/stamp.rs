use crate::cast::ToI64;
use serenity::all::{CommandDataOptionValue, CommandInteraction, UserId};
use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::prelude::{CommandOptionType, GuildId};
use sqlx::{Pool, Postgres};
use tracing::{info, warn};

const STAMP_COOLDOWN_MS: i64 = 10_000;

pub fn register_stamp() -> CreateCommand {
    CreateCommand::new("stamp")
        .description("Bookmark the current moment of a user for later clipping")
        .add_option(
            CreateCommandOption::new(CommandOptionType::User, "user", "The user to stamp")
                .required(true),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "note", "Optional note")
                .required(false),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "rewind",
                "Seconds to rewind from now (0-60)",
            )
            .min_int_value(0)
            .max_int_value(60)
            .required(false),
        )
}

pub async fn handle_stamp(
    application_command: &CommandInteraction,
    ctx: &Context,
    pool: &Pool<Postgres>,
) -> String {
    let Some(guild_id) = application_command.guild_id else {
        return "This command can only be used in a server.".to_string();
    };

    let stamper_id = application_command.user.id;

    let mut target: Option<UserId> = None;
    let mut rewind_seconds: i64 = 0;
    let mut note: Option<String> = None;

    for opt in &application_command.data.options {
        match (opt.name.as_str(), &opt.value) {
            ("user", CommandDataOptionValue::User(uid)) => target = Some(*uid),
            ("rewind", CommandDataOptionValue::Integer(i)) => rewind_seconds = *i,
            ("note", CommandDataOptionValue::String(s)) => {
                if !s.is_empty() {
                    note = Some(s.clone());
                }
            }
            _ => {}
        }
    }

    let Some(target) = target else {
        return "Missing target user.".to_string();
    };

    let channel_id = match stamper_voice_channel(ctx, guild_id, stamper_id) {
        Some(c) => c,
        None => return "You must be in a voice channel to use /stamp.".to_string(),
    };

    crate::database::user_names::observe(pool, guild_id.get(), &application_command.user, None)
        .await;
    if let Ok(target_user) = target.to_user(&ctx.http).await {
        crate::database::user_names::observe(pool, guild_id.get(), &target_user, None).await;
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let offset_ms: i32 = -(rewind_seconds as i32) * 1000;

    // TOCTOU: two /stamp calls for the same target within a few ms can both
    // pass this check and both insert. Acceptable for a 10s human-scale guard;
    // tighten with a partial unique index if strict enforcement is needed.
    match crate::database::stamps::latest_stamp_ts(pool, guild_id.to_i64(), target.to_i64())
        .await
    {
        Ok(Some(last_ts)) => {
            let delta = now_ms - last_ts;
            if delta < STAMP_COOLDOWN_MS {
                let wait = (STAMP_COOLDOWN_MS - delta + 999) / 1000;
                return format!(
                    "<@{}> was just stamped. Try again in {}s.",
                    target.get(),
                    wait
                );
            }
        }
        Ok(None) => {}
        Err(e) => {
            warn!("Failed cooldown lookup: {}", e);
            return "Database error. Try again later.".to_string();
        }
    }

    let active_file_id: Option<i64> = match crate::database::stamps::active_audio_file_id_for_stamp(
        pool,
        target.to_i64(),
        guild_id.to_i64(),
        channel_id.to_i64(),
        now_ms,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!("Failed to look up active audio_file: {}", e);
            return "Database error. Try again later.".to_string();
        }
    };

    let insert = crate::database::stamps::create_stamp(
        pool,
        guild_id.to_i64(),
        channel_id.to_i64(),
        target.to_i64(),
        stamper_id.to_i64(),
        now_ms,
        offset_ms,
        active_file_id,
        note.as_deref(),
    )
    .await;

    match insert {
        Ok(stamp_id) => {
            info!(
                stamp_id,
                target = target.get(),
                audio_file_id = ?active_file_id,
                "stamp created"
            );
            let suffix = if active_file_id.is_some() {
                ""
            } else {
                " (no active recording — stamp kept by timestamp)"
            };
            format!(
                "Stamped <@{}> at <t:{}:T>{}",
                target.get(),
                now_ms / 1000,
                suffix
            )
        }
        Err(e) => {
            warn!("Failed to insert stamp: {}", e);
            "Failed to save stamp.".to_string()
        }
    }
}

fn stamper_voice_channel(ctx: &Context, guild_id: GuildId, user_id: UserId) -> Option<u64> {
    let guild = ctx.cache.guild(guild_id)?;
    let vs = guild.voice_states.get(&user_id)?;
    vs.channel_id.map(|c| c.get())
}

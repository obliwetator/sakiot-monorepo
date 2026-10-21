//! `/recording`: lets a member opt out of (or back in to) having their voice
//! recorded in this server, and choose who may hear the recordings of it.
//! Recording is on by default. An opt-out keeps earlier recordings and takes
//! effect within a second (see `RecorderActor::apply_user_opt_outs`).
//! Recordings are shared by default with whoever may listen in their
//! channels; private keeps them, earlier ones included, to the member alone.

use serenity::all::CommandInteraction;
use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::model::prelude::CommandOptionType;
use sqlx::{Pool, Postgres};
use tracing::warn;

use crate::cast::ToI64;
use crate::database::{DbResult, opt_outs, privacy};

pub fn register_recording() -> CreateCommand {
    CreateCommand::new("recording")
        .description("Choose whether the bot records your voice in this server, and who hears it")
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "opt-out",
            "Stop recording your voice in this server",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "opt-in",
            "Record your voice in this server again",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "private",
            "Let only yourself hear the recordings of your voice in this server",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "shared",
            "Let members who may listen hear the recordings of your voice again",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "status",
            "Show whether your voice is recorded in this server, and who hears it",
        ))
}

pub async fn handle_recording(command: &CommandInteraction, pool: &Pool<Postgres>) -> String {
    let Some(guild_id) = command.guild_id else {
        return "This command can only be used in a server.".to_string();
    };
    let guild_id = guild_id.to_i64();
    let user_id = command.user.id.to_i64();
    let subcommand = command
        .data
        .options
        .first()
        .map(|option| option.name.as_str());

    let reply = match subcommand {
        Some("opt-out") => opt_outs::set_opted_out(pool, guild_id, user_id, true)
            .await
            .map(|changed| {
                if changed {
                    "You won't be recorded in this server anymore. Earlier recordings are kept. \
                     Use `/recording opt-in` to be recorded again."
                } else {
                    "You're already opted out of recording in this server."
                }
                .to_string()
            }),
        Some("opt-in") => opt_outs::set_opted_out(pool, guild_id, user_id, false)
            .await
            .map(|changed| {
                if changed {
                    "You'll be recorded in this server again."
                } else {
                    "Your voice is already recorded in this server."
                }
                .to_string()
            }),
        Some("private") => privacy::set_private(pool, guild_id, user_id, true)
            .await
            .map(|changed| {
                if changed {
                    "Only you can hear the recordings of your voice in this server now, \
                     earlier ones and clips included. Nobody else can, the server owner \
                     included. Use `/recording shared` to undo it."
                } else {
                    "The recordings of your voice in this server are already private."
                }
                .to_string()
            }),
        Some("shared") => privacy::set_private(pool, guild_id, user_id, false)
            .await
            .map(|changed| {
                if changed {
                    "Members who may listen in a channel can hear the recordings of your \
                     voice there again."
                } else {
                    "The recordings of your voice in this server are already shared."
                }
                .to_string()
            }),
        Some("status") => status(pool, guild_id, user_id).await,
        _ => return "Choose `opt-out`, `opt-in`, `private`, `shared` or `status`.".to_string(),
    };

    match reply {
        Ok(reply) => reply,
        Err(error) => {
            warn!(
                guild_id,
                user_id, "recording preference command failed: {}", error
            );
            "Could not update your recording preference. Try again later.".to_string()
        }
    }
}

async fn status(pool: &Pool<Postgres>, guild_id: i64, user_id: i64) -> DbResult<String> {
    let recorded = if opt_outs::is_opted_out(pool, guild_id, user_id).await? {
        "You're opted out of recording in this server. \
         Use `/recording opt-in` to be recorded again."
    } else {
        "Your voice is recorded in this server. Use `/recording opt-out` to stop."
    };
    let audience = if privacy::is_private(pool, guild_id, user_id).await? {
        "Only you can hear the recordings of your voice. \
         Use `/recording shared` to let members who may listen hear them."
    } else {
        "Members who may listen in a channel can hear the recordings of your voice there. \
         Use `/recording private` to keep them to yourself."
    };
    Ok(format!("{recorded}\n{audience}"))
}

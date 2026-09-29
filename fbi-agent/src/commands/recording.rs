//! `/recording`: lets a member opt out of (or back in to) having their voice
//! recorded in this server. Recording is on by default. An opt-out keeps
//! earlier recordings and takes effect within a second (see
//! `RecorderActor::apply_user_opt_outs`).

use serenity::all::CommandInteraction;
use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::model::prelude::CommandOptionType;
use sqlx::{Pool, Postgres};
use tracing::warn;

use crate::cast::ToI64;
use crate::database::opt_outs;

pub fn register_recording() -> CreateCommand {
    CreateCommand::new("recording")
        .description("Choose whether the bot records your voice in this server")
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
            "status",
            "Show whether your voice is recorded in this server",
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
            }),
        Some("opt-in") => opt_outs::set_opted_out(pool, guild_id, user_id, false)
            .await
            .map(|changed| {
                if changed {
                    "You'll be recorded in this server again."
                } else {
                    "Your voice is already recorded in this server."
                }
            }),
        Some("status") => opt_outs::is_opted_out(pool, guild_id, user_id)
            .await
            .map(|opted_out| {
                if opted_out {
                    "You're opted out of recording in this server. \
                     Use `/recording opt-in` to be recorded again."
                } else {
                    "Your voice is recorded in this server. Use `/recording opt-out` to stop."
                }
            }),
        _ => return "Choose `opt-out`, `opt-in` or `status`.".to_string(),
    };

    match reply {
        Ok(reply) => reply.to_string(),
        Err(error) => {
            warn!(
                guild_id,
                user_id, "recording preference command failed: {}", error
            );
            "Could not update your recording preference. Try again later.".to_string()
        }
    }
}

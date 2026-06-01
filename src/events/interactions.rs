use serenity::{
    all::{ButtonStyle, CommandDataOptionValue, CommandInteraction, Interaction},
    builder::{
        AutocompleteChoice, CreateActionRow, CreateAutocompleteResponse, CreateButton,
        CreateInteractionResponse, CreateInteractionResponseMessage,
    },
    client::Context,
};
use tracing::warn;

use crate::event_handler::Handler;

pub async fn interaction_create(_self: &Handler, ctx: Context, interaction: Interaction) {
    if should_skip_interaction(_self, &interaction).await {
        warn!("Ignoring interaction because another instance owns this guild");
        return;
    }

    match interaction {
        Interaction::Ping(_) => {
            warn!("Unhandled interaction type: Ping");
        }
        Interaction::Command(application_command) => {
            record_command_executed(&ctx).await;

            let mut response_msg = CreateInteractionResponseMessage::new().ephemeral(true);

            match application_command.data.name.as_str() {
                "help" => response_msg = response_msg.content(":("),
                "jam" => {
                    let (content, clip_id) = handle_jam(
                        &application_command,
                        &ctx,
                        &_self.database,
                        &_self.jam_cooldown,
                    )
                    .await;
                    response_msg = response_msg.content(content);
                    if let Some(cid) = clip_id {
                        let button = CreateButton::new(format!("jam_replay:{}", cid))
                            .label("Replay")
                            .style(ButtonStyle::Primary);
                        response_msg =
                            response_msg.components(vec![CreateActionRow::Buttons(vec![button])]);
                    }
                }
                "queue" => {
                    response_msg =
                        response_msg.content(handle_queue(&application_command, &ctx).await)
                }
                "skip" => {
                    response_msg =
                        response_msg.content(handle_skip(&application_command, &ctx).await)
                }
                "stop" => {
                    response_msg =
                        response_msg.content(handle_stop(&application_command, &ctx).await)
                }
                "join" => {
                    response_msg = response_msg
                        .content(handle_join(&application_command, &ctx, &_self.database).await)
                }
                "stamp" => {
                    response_msg = response_msg.content(
                        crate::commands::stamp::handle_stamp(
                            &application_command,
                            &ctx,
                            &_self.database,
                        )
                        .await,
                    )
                }
                other => {
                    response_msg = response_msg.content(format!(
                        "Unknown application_command with the name {}",
                        other
                    ))
                }
            }

            if let Err(why) = application_command
                .create_response(&ctx.http, CreateInteractionResponse::Message(response_msg))
                .await
            {
                warn!("Cannot respond to slash command: {}", why);
            }
        }
        Interaction::Component(component) => {
            if let Some(clip_id) = component.data.custom_id.strip_prefix("jam_replay:") {
                let user_id = component.user.id.get() as i64;
                let content = replay_clip(
                    clip_id,
                    &component.guild_id,
                    &ctx,
                    &_self.database,
                    &_self.jam_cooldown,
                    user_id,
                )
                .await;
                if let Err(why) = component
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(content)
                                .ephemeral(true),
                        ),
                    )
                    .await
                {
                    warn!("Cannot respond to replay button: {}", why);
                }
            } else {
                warn!(
                    "Unhandled interaction type: Component (id={})",
                    component.data.custom_id
                );
            }
        }
        Interaction::Autocomplete(autocomplete) => {
            if autocomplete.data.name == "jam" {
                let focused_value = autocomplete
                    .data
                    .autocomplete()
                    .map(|a| a.value)
                    .unwrap_or("");

                let guild_id = autocomplete.guild_id.map(|id| id.get() as i64).unwrap_or(0);

                let choices = get_clip_choices(focused_value, &_self.database, guild_id).await;

                if let Err(why) = autocomplete
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Autocomplete(
                            CreateAutocompleteResponse::new().set_choices(choices),
                        ),
                    )
                    .await
                {
                    warn!("Cannot respond to autocomplete: {}", why);
                }
            } else {
                warn!(
                    "Unhandled interaction type: Autocomplete (name={})",
                    autocomplete.data.name
                );
            }
        }
        Interaction::Modal(modal) => {
            warn!(
                "Unhandled interaction type: Modal (id={})",
                modal.data.custom_id
            );
        }
        _ => {
            warn!("Unhandled unknown interaction type");
        }
    }
}

async fn record_command_executed(ctx: &Context) {
    let data = ctx.data.read().await;
    if let Some(metrics) = data.get::<crate::BotMetricsKey>() {
        metrics.record_command_executed();
    }
}

async fn should_skip_interaction(handler: &Handler, interaction: &Interaction) -> bool {
    let guild_id = match interaction {
        Interaction::Command(command) => command.guild_id,
        Interaction::Component(component) => component.guild_id,
        Interaction::Autocomplete(autocomplete) => autocomplete.guild_id,
        Interaction::Modal(modal) => modal.guild_id,
        _ => None,
    };

    let Some(guild_id) = guild_id else {
        return handler.runtime.is_draining();
    };

    match crate::deployment::active_lease_owner(&handler.database, guild_id).await {
        Ok(Some(owner)) => owner != handler.runtime.config().instance_id,
        Ok(None) => handler.runtime.is_draining(),
        Err(err) => {
            warn!(
                guild_id = guild_id.get(),
                "failed to inspect voice lease before interaction routing: {}", err
            );
            handler.runtime.is_draining()
        }
    }
}

/// Read the database and return up to 25 choices matching `query`.
async fn get_clip_choices(
    query: &str,
    pool: &sqlx::Pool<sqlx::Postgres>,
    guild_id: i64,
) -> Vec<AutocompleteChoice> {
    let query_wildcard = format!("%{}%", query);

    let rows = sqlx::query!(
        "SELECT name, clip_id FROM clips WHERE guild_id = $1 AND name ILIKE $2 AND deleted_at IS NULL LIMIT 25",
        guild_id,
        query_wildcard
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut choices = Vec::new();
    for row in rows {
        if let (Some(name), clip_id) = (row.name, row.clip_id) {
            choices.push(AutocompleteChoice::new(name, clip_id));
        }
    }

    choices
}

async fn handle_jam(
    application_command: &CommandInteraction,
    ctx: &Context,
    pool: &sqlx::Pool<sqlx::Postgres>,
    cooldown: &crate::cooldown::JamCooldown,
) -> (String, Option<String>) {
    let clip_name = application_command.data.options.first().and_then(|o| {
        if let CommandDataOptionValue::String(s) = &o.value {
            Some(s.clone())
        } else {
            None
        }
    });

    let clip_name = match clip_name {
        Some(f) => f,
        None => {
            warn!("Jam command missing clip name");
            return ("Please provide a clip name.".to_string(), None);
        }
    };

    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => {
            warn!("Songbird manager not found");
            return ("Voice system is not configured.".to_string(), None);
        }
    };

    let guild_id = match application_command.guild_id {
        Some(id) => id,
        None => {
            warn!("Command not in a server");
            return (
                "This command can only be used in a server.".to_string(),
                None,
            );
        }
    };

    let user_id = application_command.user.id.get() as i64;
    match cooldown
        .check_and_record(pool, guild_id.get() as i64, user_id)
        .await
    {
        crate::cooldown::CheckResult::Allowed => {}
        crate::cooldown::CheckResult::OnCooldown { remaining_secs } => {
            return (
                format!("On cooldown — {}s remaining.", remaining_secs),
                None,
            );
        }
    }
    match crate::commands::voice_controls::play_clip(pool, &manager, guild_id, &clip_name, user_id)
        .await
    {
        Ok(msg) => (msg, Some(clip_name)),
        Err(e) => {
            warn!("Failed to play clip: {}", e);
            (e, None)
        }
    }
}

async fn replay_clip(
    clip_id: &str,
    guild_id: &Option<serenity::model::prelude::GuildId>,
    ctx: &Context,
    pool: &sqlx::Pool<sqlx::Postgres>,
    cooldown: &crate::cooldown::JamCooldown,
    user_id: i64,
) -> String {
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => return "Voice system is not configured.".to_string(),
    };

    let guild_id = match guild_id {
        Some(id) => *id,
        None => return "This command can only be used in a server.".to_string(),
    };

    match cooldown
        .check_and_record(pool, guild_id.get() as i64, user_id)
        .await
    {
        crate::cooldown::CheckResult::Allowed => {}
        crate::cooldown::CheckResult::OnCooldown { remaining_secs } => {
            return format!("On cooldown — {}s remaining.", remaining_secs);
        }
    }

    match crate::commands::voice_controls::play_clip(pool, &manager, guild_id, clip_id, user_id)
        .await
    {
        Ok(msg) => msg,
        Err(e) => e,
    }
}

async fn handle_queue(application_command: &CommandInteraction, ctx: &Context) -> String {
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => return "Voice system is not configured.".to_string(),
    };

    let guild_id = match application_command.guild_id {
        Some(id) => id,
        None => return "This command can only be used in a server.".to_string(),
    };

    crate::commands::voice_controls::queue(&manager, guild_id).await
}

async fn handle_skip(application_command: &CommandInteraction, ctx: &Context) -> String {
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => return "Voice system is not configured.".to_string(),
    };

    let guild_id = match application_command.guild_id {
        Some(id) => id,
        None => return "This command can only be used in a server.".to_string(),
    };

    crate::commands::voice_controls::skip(&manager, guild_id).await
}

async fn handle_stop(application_command: &CommandInteraction, ctx: &Context) -> String {
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => return "Voice system is not configured.".to_string(),
    };

    let guild_id = match application_command.guild_id {
        Some(id) => id,
        None => return "This command can only be used in a server.".to_string(),
    };

    crate::commands::voice_controls::stop(&manager, guild_id).await
}

async fn handle_join(
    application_command: &CommandInteraction,
    ctx: &Context,
    pool: &sqlx::Pool<sqlx::Postgres>,
) -> String {
    let guild_id = match application_command.guild_id {
        Some(id) => id,
        None => return "This command can only be used in a server.".to_string(),
    };

    crate::commands::voice_controls::join(pool, ctx, guild_id, application_command.user.id).await
}

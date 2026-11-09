//! `/recording`: lets a member opt out of (or back in to) having their voice
//! recorded in a server, and choose who may hear the recordings of it.
//! Recording is on by default. An opt-out keeps earlier recordings and takes
//! effect within a second (see `RecorderActor::apply_user_opt_outs`).
//! Recordings are shared by default with whoever may listen in their
//! channels; private keeps them, earlier ones included, to the member alone.
//!
//! Every subcommand takes an optional `server`, which also makes the command
//! usable in a direct message. Left empty it means the server the command was
//! typed in. A change aimed anywhere else asks for confirmation first.

use serenity::all::{ButtonStyle, Cache, CommandDataOptionValue, CommandInteraction, UserId};
use serenity::builder::{
    AutocompleteChoice, CreateActionRow, CreateButton, CreateCommand, CreateCommandOption,
};
use serenity::model::id::GuildId;
use serenity::model::prelude::CommandOptionType;
use sqlx::{Pool, Postgres};
use tracing::warn;

use crate::cast::ToI64;
use crate::database::{DbResult, opt_outs, privacy};
use crate::recording_notice::bold;

/// Prefix of the confirm button's custom id; `<action>:<guild id>` follows.
pub const CONFIRM_BUTTON_PREFIX: &str = "recording_confirm:";
pub const CANCEL_BUTTON_ID: &str = "recording_cancel";

/// Discord's limit on autocomplete choices.
const MAX_CHOICES: usize = 25;
/// And on the length of each one's name.
const MAX_CHOICE_NAME: usize = 100;

const FAILED: &str = "Could not update your recording preference. Try again later.";

fn subcommand(name: &str, description: &str) -> CreateCommandOption {
    CreateCommandOption::new(CommandOptionType::SubCommand, name, description).add_sub_option(
        CreateCommandOption::new(
            CommandOptionType::String,
            "server",
            "Which server. Leave empty for the one you're typing in",
        )
        .required(false)
        .set_autocomplete(true),
    )
}

pub fn register_recording() -> CreateCommand {
    CreateCommand::new("recording")
        .description("Choose whether the bot records your voice in a server, and who hears it")
        .add_option(subcommand(
            "opt-out",
            "Stop recording your voice in a server",
        ))
        .add_option(subcommand("opt-in", "Record your voice in a server again"))
        .add_option(subcommand(
            "private",
            "Let only yourself hear the recordings of your voice in a server",
        ))
        .add_option(subcommand(
            "shared",
            "Let members who may listen hear the recordings of your voice again",
        ))
        .add_option(subcommand(
            "status",
            "Show whether your voice is recorded in a server, and who hears it",
        ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    OptOut,
    OptIn,
    Private,
    Shared,
    Status,
}

impl Action {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "opt-out" => Some(Self::OptOut),
            "opt-in" => Some(Self::OptIn),
            "private" => Some(Self::Private),
            "shared" => Some(Self::Shared),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::OptOut => "opt-out",
            Self::OptIn => "opt-in",
            Self::Private => "private",
            Self::Shared => "shared",
            Self::Status => "status",
        }
    }

    /// Someone who has left isn't recorded there any more, so opting in or
    /// out means nothing, but who hears their voice still does.
    fn open_to_former(self) -> bool {
        matches!(self, Self::Private | Self::Shared | Self::Status)
    }
}

/// A server the user may set preferences for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UserGuild {
    id: i64,
    name: String,
    /// They have left, and recordings of their voice remain there.
    former: bool,
}

/// The cache's member list is the roster (see `permissions::guild_facts`).
fn member_guilds(cache: &Cache, user_id: UserId) -> Vec<UserGuild> {
    cache
        .guilds()
        .into_iter()
        .filter_map(|guild_id| {
            let guild = cache.guild(guild_id)?;
            (guild.owner_id == user_id || guild.members.contains_key(&user_id)).then(|| UserGuild {
                id: guild_id.to_i64(),
                name: guild.name.clone(),
                former: false,
            })
        })
        .collect()
}

/// Adds the guilds holding the user's recordings that they are not in.
fn with_former(mut guilds: Vec<UserGuild>, recorded: Vec<(i64, String)>) -> Vec<UserGuild> {
    for (id, name) in recorded {
        if !guilds.iter().any(|guild| guild.id == id) {
            guilds.push(UserGuild {
                id,
                name,
                former: true,
            });
        }
    }
    guilds
}

/// The servers the user is in, then those they have left. If the second
/// lookup fails they still get the first.
async fn user_guilds(cache: &Cache, pool: &Pool<Postgres>, user_id: UserId) -> Vec<UserGuild> {
    let members = member_guilds(cache, user_id);
    match privacy::recorded_guilds(pool, user_id.to_i64()).await {
        Ok(recorded) => with_former(members, recorded),
        Err(error) => {
            warn!(
                user_id = user_id.get(),
                "looking up servers with recordings failed: {}", error
            );
            members
        }
    }
}

/// The server a command acts on.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    guild_id: i64,
    /// `None` when it is the server the command was typed in.
    name: Option<String>,
    former: bool,
}

impl Target {
    fn place(&self) -> String {
        self.name
            .as_deref()
            .map_or_else(|| "this server".to_string(), bold)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Resolved {
    Guild(Target),
    /// In a direct message, with nothing to go on.
    NeedsServer,
    /// Discord lets people type anything into an autocomplete field, so this
    /// covers both a typo and a server the user isn't in, in the same words.
    Unknown,
    Ambiguous,
}

fn resolve_target(guilds: &[UserGuild], current: Option<i64>, requested: Option<&str>) -> Resolved {
    let Some(requested) = requested.map(str::trim).filter(|text| !text.is_empty()) else {
        return match current {
            Some(guild_id) => Resolved::Guild(Target {
                guild_id,
                name: None,
                former: false,
            }),
            None => Resolved::NeedsServer,
        };
    };
    let guild = match guilds
        .iter()
        .find(|guild| guild.id.to_string() == requested)
    {
        Some(guild) => guild,
        // Someone who typed the whole name without picking it from the list.
        None => {
            let wanted = requested.to_lowercase();
            let mut named = guilds
                .iter()
                .filter(|guild| guild.name.to_lowercase() == wanted);
            match (named.next(), named.next()) {
                (Some(guild), None) => guild,
                (Some(_), Some(_)) => return Resolved::Ambiguous,
                _ => return Resolved::Unknown,
            }
        }
    };
    let here = current == Some(guild.id);
    Resolved::Guild(Target {
        guild_id: guild.id,
        name: (!here).then(|| guild.name.clone()),
        // Typing in a server means being in it, whatever the cache says.
        former: guild.former && !here,
    })
}

fn left_reply(place: &str) -> String {
    format!(
        "You've left {place}, so you aren't being recorded there. You can still choose who \
         hears your recordings with `/recording private` or `/recording shared`."
    )
}

/// Changes to a server other than the current one are confirmed. Looking is
/// harmless, so `status` never is.
fn needs_confirmation(action: Action, target: &Target) -> bool {
    action != Action::Status && target.name.is_some()
}

fn confirmation_prompt(action: Action, place: &str) -> String {
    match action {
        Action::OptOut => {
            format!("Stop recording your voice in {place}? Earlier recordings are kept.")
        }
        Action::OptIn => format!("Record your voice in {place} again?"),
        Action::Private => format!(
            "Let only yourself hear the recordings of your voice in {place}, earlier ones and \
             clips included?"
        ),
        Action::Shared => {
            format!("Let members who may listen in {place} hear the recordings of your voice?")
        }
        Action::Status => String::new(),
    }
}

fn confirmation_buttons(action: Action, guild_id: i64) -> Vec<CreateActionRow> {
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!(
            "{CONFIRM_BUTTON_PREFIX}{}:{guild_id}",
            action.name()
        ))
        .label("Confirm")
        .style(ButtonStyle::Primary),
        CreateButton::new(CANCEL_BUTTON_ID)
            .label("Cancel")
            .style(ButtonStyle::Secondary),
    ])]
}

/// The part of a confirm button's custom id after [`CONFIRM_BUTTON_PREFIX`].
fn parse_confirmation(payload: &str) -> Option<(Action, i64)> {
    let (action, guild_id) = payload.split_once(':')?;
    Some((Action::parse(action)?, guild_id.parse().ok()?))
}

pub struct RecordingReply {
    pub content: String,
    pub components: Vec<CreateActionRow>,
}

impl From<String> for RecordingReply {
    fn from(content: String) -> Self {
        Self {
            content,
            components: Vec::new(),
        }
    }
}

impl From<&str> for RecordingReply {
    fn from(content: &str) -> Self {
        content.to_string().into()
    }
}

fn requested_server(value: &CommandDataOptionValue) -> Option<&str> {
    let CommandDataOptionValue::SubCommand(options) = value else {
        return None;
    };
    match &options.iter().find(|option| option.name == "server")?.value {
        CommandDataOptionValue::String(server) => Some(server),
        _ => None,
    }
}

pub async fn handle_recording(
    command: &CommandInteraction,
    cache: &Cache,
    pool: &Pool<Postgres>,
) -> RecordingReply {
    let Some((option, action)) = command
        .data
        .options
        .first()
        .and_then(|option| Some((option, Action::parse(&option.name)?)))
    else {
        return "Choose `opt-out`, `opt-in`, `private`, `shared` or `status`.".into();
    };
    let guilds = user_guilds(cache, pool, command.user.id).await;
    let current = command.guild_id.map(|guild_id| guild_id.to_i64());
    let target = match resolve_target(&guilds, current, requested_server(&option.value)) {
        Resolved::Guild(target) => target,
        Resolved::NeedsServer => {
            return "Recording is set per server. Choose which one in the `server` field.".into();
        }
        Resolved::Unknown => {
            return "That isn't one of your servers. \
                    Pick one from the list in the `server` field."
                .into();
        }
        Resolved::Ambiguous => {
            return "More than one of your servers has that name. \
                    Pick the one you mean from the list in the `server` field."
                .into();
        }
    };
    let place = target.place();
    if target.former && !action.open_to_former() {
        return left_reply(&place).into();
    }
    if needs_confirmation(action, &target) {
        return RecordingReply {
            content: confirmation_prompt(action, &place),
            components: confirmation_buttons(action, target.guild_id),
        };
    }
    apply(
        pool,
        action,
        target.guild_id,
        command.user.id.to_i64(),
        &place,
        target.former,
    )
    .await
    .into()
}

async fn apply(
    pool: &Pool<Postgres>,
    action: Action,
    guild_id: i64,
    user_id: i64,
    place: &str,
    former: bool,
) -> String {
    let reply = match action {
        Action::OptOut => opt_outs::set_opted_out(pool, guild_id, user_id, true)
            .await
            .map(|changed| {
                if changed {
                    format!(
                        "You won't be recorded in {place} anymore. Earlier recordings are kept. \
                         Use `/recording opt-in` to be recorded again."
                    )
                } else {
                    format!("You're already opted out of recording in {place}.")
                }
            }),
        Action::OptIn => opt_outs::set_opted_out(pool, guild_id, user_id, false)
            .await
            .map(|changed| {
                if changed {
                    format!("You'll be recorded in {place} again.")
                } else {
                    format!("Your voice is already recorded in {place}.")
                }
            }),
        Action::Private => privacy::set_private(pool, guild_id, user_id, true)
            .await
            .map(|changed| {
                if changed {
                    format!(
                        "Only you can hear the recordings of your voice in {place} now, \
                         earlier ones and clips included. Nobody else can, the server owner \
                         included. Use `/recording shared` to undo it."
                    )
                } else {
                    format!("The recordings of your voice in {place} are already private.")
                }
            }),
        Action::Shared => privacy::set_private(pool, guild_id, user_id, false)
            .await
            .map(|changed| {
                if changed {
                    format!(
                        "Members who may listen in {place} can hear the recordings of your \
                         voice there again."
                    )
                } else {
                    format!("The recordings of your voice in {place} are already shared.")
                }
            }),
        Action::Status => status(pool, guild_id, user_id, place, former).await,
    };

    reply.unwrap_or_else(|error| {
        warn!(
            guild_id,
            user_id, "recording preference command failed: {}", error
        );
        FAILED.to_string()
    })
}

/// The notice's button. `guild_id` comes from the button itself, because a
/// direct message has no guild; it only ever changes the presser's own row.
pub async fn opt_out_from_notice(pool: &Pool<Postgres>, guild_id: &str, user_id: i64) -> String {
    let Ok(guild_id) = guild_id.parse::<i64>() else {
        warn!(user_id, guild_id, "malformed recording opt-out button");
        return FAILED.to_string();
    };
    apply(
        pool,
        Action::OptOut,
        guild_id,
        user_id,
        "that server",
        false,
    )
    .await
}

/// The confirm button of a change aimed at another server. Membership is
/// checked again, since the prompt may have been left open for a while.
pub async fn confirm_from_button(
    cache: &Cache,
    pool: &Pool<Postgres>,
    payload: &str,
    user_id: UserId,
) -> String {
    let Some((action, guild_id)) = parse_confirmation(payload) else {
        warn!(payload, "malformed recording confirm button");
        return FAILED.to_string();
    };
    let Some(guild) = user_guilds(cache, pool, user_id)
        .await
        .into_iter()
        .find(|guild| guild.id == guild_id)
    else {
        return "You're not in that server anymore, so nothing changed.".to_string();
    };
    let place = bold(guild.name);
    if guild.former && !action.open_to_former() {
        return left_reply(&place);
    }
    apply(
        pool,
        action,
        guild_id,
        user_id.to_i64(),
        &place,
        guild.former,
    )
    .await
}

/// The `server` field's choices: the servers `subcommand` can act on that
/// match what the user has typed so far, the one they are typing in first and
/// those they have left last.
pub async fn server_choices(
    cache: &Cache,
    pool: &Pool<Postgres>,
    user_id: UserId,
    current: Option<GuildId>,
    subcommand: Option<&str>,
    typed: &str,
) -> Vec<AutocompleteChoice> {
    let open_to_former = subcommand
        .and_then(Action::parse)
        .is_some_and(Action::open_to_former);
    let guilds = if open_to_former {
        user_guilds(cache, pool, user_id).await
    } else {
        member_guilds(cache, user_id)
    };
    let current = current.map(|guild_id| guild_id.to_i64());
    rank_guilds(guilds, current, typed)
        .iter()
        .map(|guild| AutocompleteChoice::new(choice_label(guild), guild.id.to_string()))
        .collect()
}

fn choice_label(guild: &UserGuild) -> String {
    const LEFT: &str = " (left)";
    if guild.former && guild.name.chars().count() + LEFT.len() <= MAX_CHOICE_NAME {
        format!("{}{LEFT}", guild.name)
    } else {
        guild.name.clone()
    }
}

fn rank_guilds(mut guilds: Vec<UserGuild>, current: Option<i64>, typed: &str) -> Vec<UserGuild> {
    let typed = typed.trim().to_lowercase();
    guilds.retain(|guild| guild.name.to_lowercase().contains(&typed));
    guilds.sort_by_cached_key(|guild| {
        (
            Some(guild.id) != current,
            guild.former,
            guild.name.to_lowercase(),
            guild.id,
        )
    });
    guilds.truncate(MAX_CHOICES);
    guilds
}

async fn status(
    pool: &Pool<Postgres>,
    guild_id: i64,
    user_id: i64,
    place: &str,
    former: bool,
) -> DbResult<String> {
    let recorded = if former {
        format!("You've left {place}, so you aren't being recorded there.")
    } else if opt_outs::is_opted_out(pool, guild_id, user_id).await? {
        format!(
            "You're opted out of recording in {place}. Use `/recording opt-in` to be recorded again."
        )
    } else {
        format!("Your voice is recorded in {place}. Use `/recording opt-out` to stop.")
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

#[cfg(test)]
mod tests;

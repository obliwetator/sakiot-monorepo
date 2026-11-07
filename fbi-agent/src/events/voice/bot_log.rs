//! What others do to the bot in voice: a deafened bot hears nothing, so a
//! recording goes silent with no sign of why. Each deafen, undeafen, move or
//! disconnect someone else causes is logged, with who did it when Discord's
//! audit log can say. Timelines draw the deafens as spans.
//!
//! A server deafen outlives the voice session, and Discord refuses to change
//! it for a member who isn't in voice. So a deafen left behind is cleared the
//! first moment that is possible: when the bot next joins. That shows up as
//! an undeafen by the bot, so the span ends the same way.

use std::time::Duration;

use chrono::Utc;
use serenity::{
    builder::EditMember,
    client::Context,
    model::{
        guild::audit_log::{Action, AuditLogEntry, Change, MemberAction},
        id::{ChannelId, GuildId, UserId},
        prelude::VoiceState,
    },
};
use sqlx::{Pool, Postgres};
use tracing::warn;

use crate::cast::ToI64;
use crate::database::voice_events::{BotVoiceLogEntry, insert_bot_voice_log};
use crate::event_handler::Handler;

/// The audit log can trail the gateway, so it is read after each of these.
const AUDIT_LOOKUP_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];
/// How old an entry may be and still explain what just happened. Discord
/// merges a moderator's repeated moves or disconnects into their first entry,
/// so a merged one can be older than the action. Past this, saying nobody
/// is better than naming someone for an unrelated action.
const AUDIT_WINDOW_SECONDS: i64 = 5 * 60;
const AUDIT_PAGE_SIZE: u8 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnVoice {
    channel_id: Option<ChannelId>,
    deaf: bool,
}

impl From<&VoiceState> for OwnVoice {
    fn from(state: &VoiceState) -> Self {
        Self {
            channel_id: state.channel_id,
            deaf: state.deaf,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnVoiceEvent {
    Deafened {
        channel: ChannelId,
    },
    Undeafened {
        channel: ChannelId,
    },
    Moved {
        from: ChannelId,
        to: ChannelId,
    },
    Disconnected {
        from: ChannelId,
    },
    /// The bot joined still deafened from an earlier visit.
    JoinedDeafened {
        channel: ChannelId,
    },
}

/// What a voice state says happened to the bot. `intended` is where the bot
/// last put itself: a change it asked for is not an event.
fn classify(old: OwnVoice, new: OwnVoice, intended: Option<ChannelId>) -> Vec<OwnVoiceEvent> {
    let mut events = Vec::new();
    match (old.channel_id, new.channel_id) {
        (None, Some(channel)) if new.deaf => {
            events.push(OwnVoiceEvent::JoinedDeafened { channel });
        }
        (Some(from), None) if intended.is_some() => {
            events.push(OwnVoiceEvent::Disconnected { from });
        }
        (Some(from), Some(to)) if from != to && intended != Some(to) => {
            events.push(OwnVoiceEvent::Moved { from, to });
        }
        _ => {}
    }
    if old.channel_id.is_some()
        && old.deaf != new.deaf
        && let Some(channel) = new.channel_id
    {
        events.push(if new.deaf {
            OwnVoiceEvent::Deafened { channel }
        } else {
            OwnVoiceEvent::Undeafened { channel }
        });
    }
    events
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Attribution {
    /// The audit entry names the bot as its target.
    Exact(UserId),
    /// An entry of the right kind and age that names no target.
    Likely(UserId),
    Unknown,
    /// The bot may not read the audit log.
    NoAccess,
}

impl Attribution {
    fn actor(self) -> Option<UserId> {
        match self {
            Self::Exact(actor) | Self::Likely(actor) => Some(actor),
            Self::Unknown | Self::NoAccess => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Exact(_) => "exact",
            Self::Likely(_) => "likely",
            Self::Unknown => "unknown",
            Self::NoAccess => "no_access",
        }
    }
}

fn audit_action(event: OwnVoiceEvent) -> Action {
    Action::Member(match event {
        OwnVoiceEvent::Moved { .. } => MemberAction::MemberMove,
        OwnVoiceEvent::Disconnected { .. } => MemberAction::MemberDisconnect,
        OwnVoiceEvent::Deafened { .. }
        | OwnVoiceEvent::Undeafened { .. }
        | OwnVoiceEvent::JoinedDeafened { .. } => MemberAction::Update,
    })
}

/// Whether `entry`, one of the event's audit action, explains it.
fn attribute_entry(
    entry: &AuditLogEntry,
    event: OwnVoiceEvent,
    bot: UserId,
) -> Option<Attribution> {
    let names_bot = entry.target_id.is_some_and(|id| id.get() == bot.get());
    let names_another = entry.target_id.is_some() && !names_bot;
    let by_target = |actor| {
        if names_bot {
            Attribution::Exact(actor)
        } else {
            Attribution::Likely(actor)
        }
    };
    let sets_deaf = |deaf| {
        entry
            .changes
            .iter()
            .flatten()
            .any(|change| matches!(change, Change::Deaf { new, .. } if *new == Some(deaf)))
    };
    match event {
        OwnVoiceEvent::Deafened { .. } => {
            (names_bot && sets_deaf(true)).then_some(Attribution::Exact(entry.user_id))
        }
        OwnVoiceEvent::Undeafened { .. } => {
            (names_bot && sets_deaf(false)).then_some(Attribution::Exact(entry.user_id))
        }
        OwnVoiceEvent::Moved { to, .. } => {
            let into_channel = entry
                .options
                .as_ref()
                .is_some_and(|options| options.channel_id == Some(to));
            (into_channel && !names_another).then(|| by_target(entry.user_id))
        }
        OwnVoiceEvent::Disconnected { .. } => (!names_another).then(|| by_target(entry.user_id)),
        OwnVoiceEvent::JoinedDeafened { .. } => None,
    }
}

/// Who `entries`, newest first, say caused `event`. The bot counts only for an
/// undeafen, its one action here: clearing a leftover deafen.
fn find_actor(
    entries: &[AuditLogEntry],
    event: OwnVoiceEvent,
    bot: UserId,
    now_seconds: i64,
) -> Option<Attribution> {
    entries
        .iter()
        .filter(|entry| entry.user_id != bot || matches!(event, OwnVoiceEvent::Undeafened { .. }))
        .filter(|entry| {
            now_seconds.saturating_sub(entry.id.created_at().unix_timestamp())
                <= AUDIT_WINDOW_SECONDS
        })
        .find_map(|entry| attribute_entry(entry, event, bot))
}

async fn attribute(
    ctx: &Context,
    guild_id: GuildId,
    event: OwnVoiceEvent,
    bot: UserId,
) -> Attribution {
    for delay in AUDIT_LOOKUP_DELAYS {
        tokio::time::sleep(delay).await;
        match guild_id
            .audit_logs(
                &ctx.http,
                Some(audit_action(event)),
                None,
                None,
                Some(AUDIT_PAGE_SIZE),
            )
            .await
        {
            Ok(logs) => {
                if let Some(found) = find_actor(&logs.entries, event, bot, Utc::now().timestamp()) {
                    return found;
                }
            }
            Err(serenity::Error::Http(err))
                if err
                    .status_code()
                    .is_some_and(|status| status.as_u16() == 403) =>
            {
                return Attribution::NoAccess;
            }
            Err(err) => {
                warn!(
                    guild_id = guild_id.get(),
                    "failed to read the audit log: {}", err
                );
            }
        }
    }
    Attribution::Unknown
}

async fn record(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    (kind, channel, to_channel): (&'static str, ChannelId, Option<ChannelId>),
    attribution: Attribution,
    occurred_at_ms: i64,
) {
    let entry = BotVoiceLogEntry {
        guild_id: guild_id.to_i64(),
        kind,
        channel_id: channel.to_i64(),
        to_channel_id: to_channel.map(ToI64::to_i64),
        actor_id: attribution.actor().map(ToI64::to_i64),
        attribution: attribution.label(),
        occurred_at_ms,
    };
    if let Err(err) = insert_bot_voice_log(pool, entry).await {
        warn!(
            guild_id = guild_id.get(),
            kind, "failed to write the bot voice log: {}", err
        );
    }
}

async fn clear_deafen(
    ctx: &Context,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    channel: ChannelId,
    bot: UserId,
    occurred_at_ms: i64,
) {
    let edit = EditMember::new()
        .deafen(false)
        .audit_log_reason("Clearing a deafen left from an earlier voice session");
    // Success needs no row here: the voice state that follows is an undeafen.
    if let Err(err) = guild_id.edit_member(&ctx.http, bot, edit).await {
        warn!(
            guild_id = guild_id.get(),
            channel_id = channel.get(),
            "failed to clear a leftover deafen: {}",
            err
        );
        record(
            pool,
            guild_id,
            ("deafen_clear_failed", channel, None),
            Attribution::Exact(bot),
            occurred_at_ms,
        )
        .await;
    }
}

async fn handle(
    ctx: &Context,
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    event: OwnVoiceEvent,
    bot: UserId,
    occurred_at_ms: i64,
) {
    let row = match event {
        OwnVoiceEvent::JoinedDeafened { channel } => {
            clear_deafen(ctx, pool, guild_id, channel, bot, occurred_at_ms).await;
            return;
        }
        OwnVoiceEvent::Deafened { channel } => ("deafened", channel, None),
        OwnVoiceEvent::Undeafened { channel } => ("undeafened", channel, None),
        OwnVoiceEvent::Moved { from, to } => ("moved", from, Some(to)),
        OwnVoiceEvent::Disconnected { from } => ("disconnected", from, None),
    };
    let attribution = attribute(ctx, guild_id, event, bot).await;
    record(pool, guild_id, row, attribution, occurred_at_ms).await;
}

/// Logs what a voice state of the bot's own says someone did to it, and clears
/// a leftover deafen. Runs off the event: the audit log needs a moment.
pub(super) async fn own_voice_state(
    handler: &Handler,
    ctx: &Context,
    old: Option<&VoiceState>,
    new: &VoiceState,
) {
    let Some(guild_id) = new.guild_id else {
        return;
    };
    let Some(registry) = super::coordinator_registry(&ctx.data).await else {
        return;
    };
    let coordinator = registry.guild(guild_id);
    let old = old.map_or(
        OwnVoice {
            channel_id: None,
            deaf: false,
        },
        OwnVoice::from,
    );
    let events = classify(old, OwnVoice::from(new), coordinator.intended_channel());
    if events.iter().any(|event| {
        matches!(
            event,
            OwnVoiceEvent::Moved { .. } | OwnVoiceEvent::Disconnected { .. }
        )
    }) {
        coordinator.intend_channel(new.channel_id);
    }
    // During a deploy two instances see every event; only one may log it.
    if events.is_empty() || super::should_skip_voice_state_for_lease(handler, Some(guild_id)).await
    {
        return;
    }

    let bot = new.user_id;
    let occurred_at_ms = Utc::now().timestamp_millis();
    let (ctx, pool) = (ctx.clone(), handler.database.clone());
    tokio::spawn(async move {
        for event in events {
            handle(&ctx, &pool, guild_id, event, bot, occurred_at_ms).await;
        }
    });
}

#[cfg(test)]
mod tests;

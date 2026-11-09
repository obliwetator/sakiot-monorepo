//! Delivers the notices queued in `database::recording_notices`, which tell a
//! member, once per guild, that their voice is recorded there and how to stop.
//!
//! The guild picks a direct message or the voice channel's text chat, and the
//! other is the fallback: what matters is that the member hears about it. Only
//! one notice goes out per tick, to stay well inside what Discord tolerates
//! from a bot opening direct messages. Delivery is at least once: dying
//! between the send and recording it repeats the notice.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{ButtonStyle, Cache, ChannelId, GuildId, Http, Message, User, UserId};
use serenity::builder::{CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage};
use serenity::utils::MessageBuilder;
use sqlx::{Pool, Postgres};
use tokio::time::MissedTickBehavior;
use tracing::warn;

use crate::cast::ToI64;
use crate::database::DbResult;
use crate::database::opt_outs;
use crate::database::recording_notices::{self, Delivery, DueNotice, MAX_ATTEMPTS};
use crate::runtime::RuntimeState;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Prefix of the opt-out button's custom id; the guild id follows it, because
/// a direct message has no guild of its own.
pub const OPT_OUT_BUTTON_PREFIX: &str = "recording_opt_out:";

pub(crate) struct NoticeSender {
    pub pool: Pool<Postgres>,
    pub http: Arc<Http>,
    pub cache: Arc<Cache>,
    pub website: Option<String>,
}

/// Exits on shutdown. A notice it never reached stays queued for the next
/// instance, so it needs no supervision.
pub(crate) fn spawn(
    sender: NoticeSender,
    runtime: Arc<RuntimeState>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if !runtime.is_draining()
                        && let Err(error) = sender.deliver_next().await
                    {
                        warn!("recording notice delivery failed: {}", error);
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }
    })
}

impl NoticeSender {
    async fn deliver_next(&self) -> DbResult<()> {
        match recording_notices::claim_due(&self.pool).await? {
            Some(notice) => self.deliver(&notice).await,
            None => Ok(()),
        }
    }

    async fn deliver(&self, notice: &DueNotice) -> DbResult<()> {
        let user = match self.http.get_user(UserId::new(notice.user_id as u64)).await {
            Ok(user) => user,
            Err(error) => {
                let give_up = is_rejection(&error) || notice.attempts >= MAX_ATTEMPTS;
                return recording_notices::record_failure(
                    &self.pool,
                    notice,
                    &error.to_string(),
                    give_up,
                )
                .await;
            }
        };
        // Bots can reach the recorder, and someone who has already opted out
        // doesn't need telling how to.
        if user.bot || opt_outs::is_opted_out(&self.pool, notice.guild_id, notice.user_id).await? {
            return recording_notices::mark_skipped(&self.pool, notice).await;
        }
        let guild_name = self
            .cache
            .guild(GuildId::new(notice.guild_id as u64))
            .map(|guild| guild.name.clone());
        let Some(guild_name) = guild_name else {
            // Normal just after a start, before the cache has filled.
            return recording_notices::record_failure(
                &self.pool,
                notice,
                "guild is not in the cache",
                notice.attempts >= MAX_ATTEMPTS,
            )
            .await;
        };

        let mut refused = String::new();
        for &via in routes(notice.delivery) {
            let message = build_message(notice, via, &guild_name, self.website.as_deref());
            match self.send(notice, &user, via, message).await {
                Ok(sent) => {
                    return recording_notices::mark_sent(&self.pool, notice, via, sent.id.to_i64())
                        .await;
                }
                Err(error) if is_rejection(&error) => {
                    warn!(
                        guild_id = notice.guild_id,
                        user_id = notice.user_id,
                        route = ?via,
                        "recording notice refused: {}",
                        error
                    );
                    refused = error.to_string();
                }
                // Discord or the network: the same route may work later.
                Err(error) => {
                    return recording_notices::record_failure(
                        &self.pool,
                        notice,
                        &error.to_string(),
                        notice.attempts >= MAX_ATTEMPTS,
                    )
                    .await;
                }
            }
        }
        recording_notices::record_failure(&self.pool, notice, &refused, true).await
    }

    async fn send(
        &self,
        notice: &DueNotice,
        user: &User,
        via: Delivery,
        message: CreateMessage,
    ) -> serenity::Result<Message> {
        match via {
            Delivery::Dm => user.direct_message(self.http.as_ref(), message).await,
            Delivery::Channel => {
                ChannelId::new(notice.channel_id as u64)
                    .send_message(self.http.as_ref(), message)
                    .await
            }
        }
    }
}

/// The routes to try, in order, for a guild's preference.
fn routes(preference: Delivery) -> &'static [Delivery] {
    match preference {
        Delivery::Dm => &[Delivery::Dm, Delivery::Channel],
        Delivery::Channel => &[Delivery::Channel, Delivery::Dm],
    }
}

/// Whether Discord refused the request outright (closed DMs, missing
/// permissions, a deleted channel), so repeating it won't help.
fn is_rejection(error: &serenity::Error) -> bool {
    match error {
        serenity::Error::Http(http) => http
            .status_code()
            .is_some_and(|status| refused_status(status.as_u16())),
        _ => false,
    }
}

fn refused_status(status: u16) -> bool {
    (400..500).contains(&status) && status != 429
}

fn build_message(
    notice: &DueNotice,
    via: Delivery,
    guild_name: &str,
    website: Option<&str>,
) -> CreateMessage {
    let mention = (via == Delivery::Channel).then_some(notice.user_id);
    // Only the member is ever pinged, whatever the guild's name contains.
    let allowed_mentions = match mention {
        Some(user_id) => CreateAllowedMentions::new().users([UserId::new(user_id as u64)]),
        None => CreateAllowedMentions::new(),
    };
    let button = CreateButton::new(format!("{OPT_OUT_BUTTON_PREFIX}{}", notice.guild_id))
        .label("Stop recording me")
        .style(ButtonStyle::Primary);
    CreateMessage::new()
        .content(notice_text(guild_name, notice.guild_id, website, mention))
        .allowed_mentions(allowed_mentions)
        .components(vec![CreateActionRow::Buttons(vec![button])])
}

pub(crate) fn bold(text: impl Into<String>) -> String {
    MessageBuilder::new().push_bold_safe(text.into()).build()
}

fn notice_text(
    guild_name: &str,
    guild_id: i64,
    website: Option<&str>,
    mention: Option<i64>,
) -> String {
    let mut text = String::new();
    if let Some(user_id) = mention {
        text.push_str(&format!("<@{user_id}> "));
    }
    text.push_str(&format!(
        "{} records its voice channels, and **your voice is recorded there by default**. \
         This applies to this server only, and you can stop it at any time:\n\
         - Press **Stop recording me** below.\n\
         - Or run `/recording opt-out` in the server.\n",
        bold(guild_name)
    ));
    if let Some(website) = website {
        text.push_str(&format!(
            "- Or open {website}/dashboard/{guild_id}, click your avatar at the top right and \
             untick {}.\n",
            bold(format!("Record my voice in {guild_name}"))
        ));
    }
    text.push_str(
        "\nOpting out takes effect within a second and keeps the recordings made so far. \
         To let only yourself hear them, run `/recording private` in the server",
    );
    if website.is_some() {
        text.push_str(&format!(
            " or tick {} in the same menu",
            bold(format!(
                "Only I can listen to my recordings in {guild_name}"
            ))
        ));
    }
    text.push('.');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preference_falls_back_to_the_other_route() {
        assert_eq!(routes(Delivery::Dm), &[Delivery::Dm, Delivery::Channel][..]);
        assert_eq!(
            routes(Delivery::Channel),
            &[Delivery::Channel, Delivery::Dm][..]
        );
    }

    #[test]
    fn only_refusals_stop_a_route_being_retried() {
        assert!(refused_status(403));
        assert!(refused_status(404));
        assert!(!refused_status(429));
        assert!(!refused_status(500));
        assert!(!refused_status(503));
    }

    #[test]
    fn the_notice_says_it_is_opt_out_and_how_to_stop() {
        let text = notice_text("Rouvas", 42, Some("https://example.test"), None);
        assert!(text.contains("recorded there by default"));
        assert!(text.contains("Stop recording me"));
        assert!(text.contains("/recording opt-out"));
        assert!(text.contains("https://example.test/dashboard/42"));
        assert!(text.contains("Record my voice in Rouvas"));
    }

    #[test]
    fn the_notice_omits_the_website_when_none_is_configured() {
        let text = notice_text("Rouvas", 42, None, None);
        assert!(!text.contains("dashboard"));
        assert!(!text.contains("same menu"));
        assert!(text.contains("/recording private"));
    }

    #[test]
    fn only_a_channel_notice_mentions_the_member() {
        assert!(notice_text("Rouvas", 42, None, Some(7)).starts_with("<@7> "));
        assert!(!notice_text("Rouvas", 42, None, None).contains("<@"));
    }

    #[test]
    fn the_longest_notice_fits_in_a_discord_message() {
        let name = "w".repeat(100);
        let website = format!("https://{}.example.test", "a".repeat(60));
        let text = notice_text(&name, i64::MAX, Some(&website), Some(i64::MAX));
        assert!(text.chars().count() <= 2000, "{}", text.chars().count());
    }
}

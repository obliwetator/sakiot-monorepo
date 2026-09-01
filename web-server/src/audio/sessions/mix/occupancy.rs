//! Reconstructs bot channel occupancy around a selected recording timeline.

use std::collections::HashSet;

use actix_web::web;
use chrono::Utc;
use sqlx::{Pool, Postgres, Row};

use crate::errors::AppError;

use super::super::{AudioFragment, SessionAccess};
use super::MixInterval;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MixWindow {
    pub(super) channel_id: i64,
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
}

impl MixWindow {
    fn new(channel_id: i64, start_ms: i64, end_ms: i64) -> Option<Self> {
        (end_ms > start_ms).then_some(Self {
            channel_id,
            start_ms,
            end_ms,
        })
    }

    pub(super) fn interval(&self) -> MixInterval {
        MixInterval {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BotConnectionEvent {
    pub(super) started_ms: i64,
    pub(super) completed_ms: i64,
    pub(super) to_channel_id: Option<i64>,
    pub(super) outcome: String,
}

#[derive(Clone, Debug)]
pub(super) struct OccupancyWindow {
    pub(super) channel_id: i64,
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
    pub(super) episode_id: i64,
}

fn is_successful_connection_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "joined" | "rejoined" | "switched" | "already_in_channel" | "disconnected"
    ) || outcome == "switched_after_join_error"
}

pub(super) fn build_occupancy_windows(
    events: &[BotConnectionEvent],
    horizon_end_ms: i64,
) -> Vec<OccupancyWindow> {
    let mut windows = Vec::new();
    let mut current: Option<(i64, i64, i64)> = None;
    let mut next_episode_id = 0;

    for event in events {
        if !is_successful_connection_outcome(&event.outcome) {
            continue;
        }
        let operation_start_ms = event.started_ms.min(event.completed_ms);
        let operation_end_ms = event.completed_ms.max(event.started_ms);
        match event.to_channel_id {
            Some(channel_id) => {
                let episode_id =
                    if let Some((current_channel_id, episode_id, current_start_ms)) = current {
                        if current_channel_id == channel_id {
                            // A successful health-check/rejoin to the same channel
                            // does not split the bot's connected presence.
                            continue;
                        }
                        if let Some(window) =
                            MixWindow::new(current_channel_id, current_start_ms, operation_start_ms)
                        {
                            windows.push(OccupancyWindow {
                                channel_id: window.channel_id,
                                start_ms: window.start_ms,
                                end_ms: window.end_ms,
                                episode_id,
                            });
                        }
                        episode_id
                    } else {
                        next_episode_id += 1;
                        next_episode_id
                    };
                current = Some((channel_id, episode_id, operation_end_ms));
            }
            None => {
                if let Some((channel_id, episode_id, current_start_ms)) = current.take()
                    && let Some(window) =
                        MixWindow::new(channel_id, current_start_ms, operation_start_ms)
                {
                    windows.push(OccupancyWindow {
                        channel_id: window.channel_id,
                        start_ms: window.start_ms,
                        end_ms: window.end_ms,
                        episode_id,
                    });
                }
            }
        }
    }

    if let Some((channel_id, episode_id, current_start_ms)) = current
        && let Some(window) = MixWindow::new(channel_id, current_start_ms, horizon_end_ms)
    {
        windows.push(OccupancyWindow {
            channel_id: window.channel_id,
            start_ms: window.start_ms,
            end_ms: window.end_ms,
            episode_id,
        });
    }
    windows
}

pub(super) fn select_occupancy_windows(
    occupancy: Vec<OccupancyWindow>,
    selected_start_ms: i64,
    selected_end_ms: i64,
    selected_channels: &HashSet<i64>,
) -> Vec<MixWindow> {
    let selected_episodes = occupancy
        .iter()
        .filter(|window| {
            selected_channels.contains(&window.channel_id)
                && window.start_ms < selected_end_ms
                && window.end_ms > selected_start_ms
        })
        .map(|window| window.episode_id)
        .collect::<HashSet<_>>();
    occupancy
        .into_iter()
        .filter(|window| {
            selected_episodes.contains(&window.episode_id)
                && selected_channels.contains(&window.channel_id)
        })
        .filter_map(|window| MixWindow::new(window.channel_id, window.start_ms, window.end_ms))
        .collect()
}

pub(super) async fn load_bot_occupancy_windows(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    selected_fragments: &[AudioFragment],
    selected_timeline_end_ms: i64,
) -> Result<Vec<MixWindow>, AppError> {
    let now_ms = Utc::now().timestamp_millis();
    let rows = sqlx::query(
        "SELECT ((EXTRACT(EPOCH FROM started_at) * 1000)::bigint) AS started_ms,
                ((EXTRACT(EPOCH FROM completed_at) * 1000)::bigint) AS completed_ms,
                to_channel_id,
                outcome
           FROM voice_connection_events
          WHERE guild_id = $1
            AND completed_at <= now()
          ORDER BY completed_at, id",
    )
    .bind(access.guild_id)
    .fetch_all(pool.get_ref())
    .await?;
    let events = rows
        .into_iter()
        .map(|row| {
            Ok(BotConnectionEvent {
                started_ms: row.try_get("started_ms")?,
                completed_ms: row.try_get("completed_ms")?,
                to_channel_id: row.try_get("to_channel_id")?,
                outcome: row.try_get("outcome")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let occupancy = build_occupancy_windows(&events, now_ms.max(selected_timeline_end_ms));
    let selected_start_ms = access.started_at_ms;
    let selected_channels = selected_fragments
        .iter()
        .filter(|fragment| fragment.channel_id != 0)
        .map(|fragment| fragment.channel_id)
        .chain(std::iter::once(access.starting_channel_id))
        .collect::<HashSet<_>>();
    Ok(select_occupancy_windows(
        occupancy,
        selected_start_ms,
        selected_timeline_end_ms,
        &selected_channels,
    ))
}

pub(super) fn fallback_mix_windows(
    access: &SessionAccess,
    fragments: &[AudioFragment],
    timeline_end_ms: i64,
) -> Vec<MixWindow> {
    fragments
        .iter()
        .filter(|fragment| fragment.channel_id != 0)
        .filter_map(|fragment| {
            MixWindow::new(
                fragment.channel_id,
                fragment.start_ms.max(access.started_at_ms),
                fragment
                    .end_ms
                    .unwrap_or(timeline_end_ms)
                    .min(timeline_end_ms),
            )
        })
        .collect()
}

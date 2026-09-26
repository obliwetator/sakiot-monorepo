//! The recorder's view of its voice connection, as an explicit state machine.
//!
//! Transitions are pure: each one updates [`Link`] and returns the
//! [`Departure`] the actor must record, if any. The actor performs the database
//! side effects, so every edge here is testable without Discord or PostgreSQL.

use serenity::model::id::ChannelId;

use crate::events::voice_receiver::disconnect::RECOVERABLE_DISCONNECT_TIMEOUT_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedHandoff {
    pub(super) from_channel_id: ChannelId,
    /// `None` when the bot is leaving voice rather than moving.
    pub(super) to_channel_id: Option<ChannelId>,
}

impl PlannedHandoff {
    fn departure(self) -> Departure {
        Departure {
            from_channel_id: self.from_channel_id,
            to_channel_id: self.to_channel_id,
            reason: if self.to_channel_id.is_some() {
                "handoff"
            } else {
                "bot_departure"
            },
            starts_grace: false,
        }
    }
}

/// How long a dropped driver has to reconnect before the call is torn down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReconnectWindow {
    deadline_ms: i64,
}

impl ReconnectWindow {
    fn opened_at(at_ms: i64) -> Self {
        Self {
            deadline_ms: at_ms.saturating_add(RECOVERABLE_DISCONNECT_TIMEOUT_MS as i64),
        }
    }
}

/// Writers to close and logical sessions to pause when the bot stops
/// recording a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Departure {
    pub(super) from_channel_id: ChannelId,
    pub(super) to_channel_id: Option<ChannelId>,
    pub(super) reason: &'static str,
    /// Whether paused users enter the pending grace period.
    pub(super) starts_grace: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Link {
    /// The driver is connected (or left for good) with no move in progress.
    #[default]
    Connected,
    /// A move or leave was announced; the driver has not dropped yet.
    HandoffPlanned(PlannedHandoff),
    /// The driver dropped and may still reconnect before the window closes.
    /// `handoff` is kept when the drop happened mid-handoff so that a session
    /// end during the window is still recorded as that handoff.
    Reconnecting {
        window: ReconnectWindow,
        handoff: Option<PlannedHandoff>,
    },
    /// The voice session ended; the run loop is exiting.
    Ended,
}

impl Link {
    fn handoff(self) -> Option<PlannedHandoff> {
        match self {
            Self::HandoffPlanned(handoff) => Some(handoff),
            Self::Reconnecting { handoff, .. } => handoff,
            Self::Connected | Self::Ended => None,
        }
    }

    pub(super) fn begin_handoff(&mut self, handoff: PlannedHandoff) {
        *self = match *self {
            Self::Connected | Self::HandoffPlanned(_) => Self::HandoffPlanned(handoff),
            Self::Reconnecting { window, .. } => Self::Reconnecting {
                window,
                handoff: Some(handoff),
            },
            Self::Ended => Self::Ended,
        };
    }

    pub(super) fn cancel_handoff(&mut self) {
        *self = match *self {
            Self::HandoffPlanned(_) => Self::Connected,
            Self::Reconnecting { window, .. } => Self::Reconnecting {
                window,
                handoff: None,
            },
            other => other,
        };
    }

    /// The driver connected (to `channel_id`, possibly a new one): any move is
    /// complete and any reconnect window is closed.
    pub(super) fn connected(&mut self) {
        if *self != Self::Ended {
            *self = Self::Connected;
        }
    }

    pub(super) fn driver_disconnected(
        &mut self,
        channel_id: ChannelId,
        recoverable: bool,
        finalize_empty_channel: bool,
        at_ms: i64,
    ) -> Option<Departure> {
        if *self == Self::Ended {
            return None;
        }

        // A planned move explains the drop, whatever the driver says about it.
        // Only a move to another channel waits for the driver to come back; a
        // planned leave stays planned until the session ends.
        if let Some(handoff) = self.handoff() {
            if handoff.to_channel_id.is_some() {
                *self = Self::Reconnecting {
                    window: ReconnectWindow::opened_at(at_ms),
                    handoff: Some(handoff),
                };
            }
            return Some(handoff.departure());
        }

        if recoverable {
            // Repeated drops inside one window neither extend it nor pause twice.
            if matches!(self, Self::Reconnecting { .. }) {
                return None;
            }
            *self = Self::Reconnecting {
                window: ReconnectWindow::opened_at(at_ms),
                handoff: None,
            };
            return Some(Departure {
                from_channel_id: channel_id,
                to_channel_id: Some(channel_id),
                reason: "network",
                starts_grace: true,
            });
        }

        *self = Self::Connected;
        Some(Departure {
            from_channel_id: channel_id,
            to_channel_id: None,
            reason: if finalize_empty_channel {
                "empty_channel"
            } else {
                "bot_departure"
            },
            starts_grace: false,
        })
    }

    pub(super) fn is_reconnecting(self) -> bool {
        matches!(self, Self::Reconnecting { .. })
    }

    pub(super) fn reconnect_deadline_passed(self, now_ms: i64) -> bool {
        matches!(self, Self::Reconnecting { window, .. } if now_ms >= window.deadline_ms)
    }

    /// The stale call was torn down after its reconnect window closed.
    pub(super) fn torn_down(&mut self) {
        if *self != Self::Ended {
            *self = Self::Connected;
        }
    }

    /// Ends the session, returning the departure to record, or `None` if it
    /// already ended.
    pub(super) fn end_session(&mut self, channel_id: ChannelId) -> Option<Departure> {
        if *self == Self::Ended {
            return None;
        }
        let departure = self
            .handoff()
            .map(PlannedHandoff::departure)
            .unwrap_or(Departure {
                from_channel_id: channel_id,
                to_channel_id: None,
                reason: "bot_departure",
                starts_grace: false,
            });
        *self = Self::Ended;
        Some(departure)
    }
}

#[cfg(test)]
mod tests {
    use serenity::model::id::ChannelId;

    use super::{Departure, Link, PlannedHandoff, ReconnectWindow};
    use crate::events::voice_receiver::disconnect::RECOVERABLE_DISCONNECT_TIMEOUT_MS;

    const TIMEOUT: i64 = RECOVERABLE_DISCONNECT_TIMEOUT_MS as i64;

    fn channel(id: u64) -> ChannelId {
        ChannelId::new(id)
    }

    fn move_to(to: u64) -> PlannedHandoff {
        PlannedHandoff {
            from_channel_id: channel(1),
            to_channel_id: Some(channel(to)),
        }
    }

    fn leave() -> PlannedHandoff {
        PlannedHandoff {
            from_channel_id: channel(1),
            to_channel_id: None,
        }
    }

    fn reconnecting(at_ms: i64, handoff: Option<PlannedHandoff>) -> Link {
        Link::Reconnecting {
            window: ReconnectWindow::opened_at(at_ms),
            handoff,
        }
    }

    #[test]
    fn a_recoverable_drop_opens_one_window_and_pauses_once() {
        let mut link = Link::Connected;
        assert_eq!(
            link.driver_disconnected(channel(1), true, false, 1_000),
            Some(Departure {
                from_channel_id: channel(1),
                to_channel_id: Some(channel(1)),
                reason: "network",
                starts_grace: true,
            })
        );
        assert_eq!(link, reconnecting(1_000, None));

        assert_eq!(
            link.driver_disconnected(channel(1), true, false, 5_000),
            None
        );
        assert_eq!(
            link,
            reconnecting(1_000, None),
            "a repeat drop must not extend the window"
        );
    }

    #[test]
    fn a_reconnect_before_the_deadline_closes_the_window() {
        let mut link = reconnecting(1_000, None);
        assert!(!link.reconnect_deadline_passed(1_000 + TIMEOUT - 1));
        link.connected();
        assert_eq!(link, Link::Connected);
        assert!(!link.reconnect_deadline_passed(1_000 + TIMEOUT));
    }

    #[test]
    fn the_deadline_fires_once_until_a_new_drop() {
        let mut link = reconnecting(1_000, None);
        assert!(link.reconnect_deadline_passed(1_000 + TIMEOUT));
        link.torn_down();
        assert!(!link.reconnect_deadline_passed(1_000 + TIMEOUT + 1));
        assert!(!link.is_reconnecting());
    }

    #[test]
    fn an_unrecoverable_drop_departs_and_clears_the_window() {
        let mut link = reconnecting(1_000, None);
        assert_eq!(
            link.driver_disconnected(channel(1), false, true, 2_000),
            Some(Departure {
                from_channel_id: channel(1),
                to_channel_id: None,
                reason: "empty_channel",
                starts_grace: false,
            })
        );
        assert_eq!(link, Link::Connected);

        let mut link = Link::Connected;
        assert_eq!(
            link.driver_disconnected(channel(1), false, false, 2_000)
                .map(|departure| departure.reason),
            Some("bot_departure")
        );
    }

    #[test]
    fn a_planned_move_explains_the_drop_and_waits_for_the_new_channel() {
        let mut link = Link::Connected;
        link.begin_handoff(move_to(2));
        assert_eq!(link, Link::HandoffPlanned(move_to(2)));

        // Even an "unrecoverable" drop is the planned move, not a departure.
        assert_eq!(
            link.driver_disconnected(channel(1), false, false, 3_000),
            Some(Departure {
                from_channel_id: channel(1),
                to_channel_id: Some(channel(2)),
                reason: "handoff",
                starts_grace: false,
            })
        );
        assert_eq!(link, reconnecting(3_000, Some(move_to(2))));

        link.connected();
        assert_eq!(link, Link::Connected);
    }

    #[test]
    fn a_planned_leave_stays_planned_without_a_reconnect_window() {
        let mut link = Link::HandoffPlanned(leave());
        assert_eq!(
            link.driver_disconnected(channel(1), true, false, 3_000)
                .map(|departure| departure.reason),
            Some("bot_departure")
        );
        assert_eq!(link, Link::HandoffPlanned(leave()));
        assert!(!link.is_reconnecting());
    }

    #[test]
    fn handoffs_planned_or_cancelled_mid_window_keep_the_deadline() {
        let mut link = reconnecting(1_000, None);
        link.begin_handoff(move_to(2));
        assert_eq!(link, reconnecting(1_000, Some(move_to(2))));
        link.cancel_handoff();
        assert_eq!(link, reconnecting(1_000, None));

        let mut link = Link::HandoffPlanned(move_to(2));
        link.cancel_handoff();
        assert_eq!(link, Link::Connected);
    }

    #[test]
    fn a_session_end_records_the_pending_handoff() {
        let mut link = reconnecting(1_000, Some(move_to(2)));
        assert_eq!(link.end_session(channel(1)), Some(move_to(2).departure()));
        assert_eq!(link, Link::Ended);

        let mut link = Link::Connected;
        assert_eq!(
            link.end_session(channel(7)),
            Some(Departure {
                from_channel_id: channel(7),
                to_channel_id: None,
                reason: "bot_departure",
                starts_grace: false,
            })
        );
    }

    #[test]
    fn an_ended_session_ignores_every_later_event() {
        let mut link = Link::Ended;
        assert_eq!(link.end_session(channel(1)), None);
        assert_eq!(
            link.driver_disconnected(channel(1), true, false, 1_000),
            None
        );
        link.begin_handoff(move_to(2));
        link.connected();
        link.torn_down();
        assert_eq!(link, Link::Ended);
    }
}

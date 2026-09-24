//! Whether the guild's recording policy currently lets the actor record the
//! channel it is connected to.
//!
//! The actor runs the policy query and the departure bookkeeping; this type
//! owns the state those depend on, so suspension, resumption and the speakers
//! to reopen can only change together.

use std::sync::Arc;

use crate::events::voice_receiver::recordings::{RecorderStats, SuspendedSpeakers};

/// Minimum spacing between policy queries driven by voice ticks.
const CHECK_INTERVAL_MS: i64 = 1_000;

enum State {
    Allowed,
    /// The channel is excluded, or its policy could not be read (fail
    /// closed). No writer may be open; `speakers` are reopened on resume.
    Suspended {
        speakers: SuspendedSpeakers,
    },
}

pub(super) struct RecordingPolicy {
    state: State,
    last_check_ms: i64,
    /// Published copy of the suspension flag (see `RecorderStats`).
    stats: Arc<RecorderStats>,
}

impl RecordingPolicy {
    pub(super) fn new(stats: Arc<RecorderStats>) -> Self {
        stats.set_policy_suspended(false);
        Self {
            state: State::Allowed,
            last_check_ms: 0,
            stats,
        }
    }

    fn set(&mut self, state: State) {
        self.stats
            .set_policy_suspended(matches!(state, State::Suspended { .. }));
        self.state = state;
    }

    pub(super) fn is_suspended(&self) -> bool {
        matches!(self.state, State::Suspended { .. })
    }

    /// Whether a tick at `at_ms` should re-query the policy. Claims the check
    /// when it returns `true`.
    pub(super) fn check_due(&mut self, at_ms: i64) -> bool {
        if at_ms.saturating_sub(self.last_check_ms) < CHECK_INTERVAL_MS {
            return false;
        }
        self.last_check_ms = at_ms;
        true
    }

    /// Remembers a speaker while suspended, returning `true` if it did (the
    /// caller must then not open a writer).
    pub(super) fn remember_speaker(&mut self, user_id: u64, ssrc: u32) -> bool {
        match &mut self.state {
            State::Suspended { speakers } => {
                speakers.observe(user_id, ssrc);
                true
            }
            State::Allowed => false,
        }
    }

    /// Suspends recording, remembering the users whose writers are about to be
    /// closed. Returns how many were remembered, or `None` if recording was
    /// already suspended.
    pub(super) fn suspend(
        &mut self,
        open_writers: impl IntoIterator<Item = (u64, u32)>,
    ) -> Option<usize> {
        if self.is_suspended() {
            return None;
        }
        let mut speakers = SuspendedSpeakers::default();
        speakers.seed(open_writers);
        let count = speakers.count();
        self.set(State::Suspended { speakers });
        Some(count)
    }

    /// Allows recording again, returning the speakers to reopen, or `None` if
    /// recording was not suspended.
    pub(super) fn allow(&mut self) -> Option<Vec<(u64, u32)>> {
        let State::Suspended { speakers } = &mut self.state else {
            return None;
        };
        let speakers = speakers.take_all();
        self.set(State::Allowed);
        Some(speakers)
    }

    /// The actor moved to a channel whose policy was just evaluated. Speakers
    /// from the previous channel are meaningless there, and the next tick must
    /// not skip its check because of a timestamp from the previous channel.
    pub(super) fn entered_channel(&mut self, allowed: bool) {
        self.last_check_ms = 0;
        self.set(if allowed {
            State::Allowed
        } else {
            State::Suspended {
                speakers: SuspendedSpeakers::default(),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::RecordingPolicy;
    use crate::events::voice_receiver::recordings::RecorderStats;

    fn policy() -> (RecordingPolicy, Arc<RecorderStats>) {
        let stats = Arc::new(RecorderStats::default());
        (RecordingPolicy::new(Arc::clone(&stats)), stats)
    }

    #[test]
    fn checks_are_throttled_to_one_per_second() {
        let (mut policy, _) = policy();
        assert!(policy.check_due(10_000));
        assert!(!policy.check_due(10_999));
        assert!(policy.check_due(11_000));
    }

    #[test]
    fn suspension_is_published_and_idempotent() {
        let (mut policy, stats) = policy();
        assert_eq!(policy.suspend([(7, 100), (8, 200)]), Some(2));
        assert!(policy.is_suspended());
        assert!(stats.policy_suspended());

        assert_eq!(policy.suspend([(9, 300)]), None, "already suspended");
    }

    #[test]
    fn speakers_are_only_remembered_while_suspended() {
        let (mut policy, _) = policy();
        assert!(
            !policy.remember_speaker(7, 100),
            "allowed: open a writer instead"
        );

        policy.suspend([(7, 100)]);
        assert!(policy.remember_speaker(7, 101));
        assert!(policy.remember_speaker(8, 200));

        let mut speakers = policy.allow().expect("was suspended");
        speakers.sort_unstable();
        assert_eq!(speakers, vec![(7, 101), (8, 200)]);
    }

    #[test]
    fn allowing_is_published_and_idempotent() {
        let (mut policy, stats) = policy();
        assert_eq!(policy.allow(), None, "never suspended");

        policy.suspend([]);
        assert_eq!(policy.allow(), Some(vec![]));
        assert!(!policy.is_suspended());
        assert!(!stats.policy_suspended());
        assert_eq!(policy.allow(), None);
    }

    #[test]
    fn entering_a_channel_forgets_speakers_and_rechecks_immediately() {
        let (mut policy, stats) = policy();
        assert!(policy.check_due(50_000));
        policy.suspend([(7, 100)]);

        policy.entered_channel(false);
        assert!(stats.policy_suspended());
        assert_eq!(
            policy.allow(),
            Some(vec![]),
            "previous channel's speakers dropped"
        );

        policy.entered_channel(true);
        assert!(!stats.policy_suspended());
        assert!(policy.check_due(50_001), "throttle reset on channel change");
    }
}

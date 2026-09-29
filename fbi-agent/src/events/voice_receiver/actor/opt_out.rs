//! Speakers the actor is not recording because they opted out.
//!
//! Each is remembered with its SSRC so a writer can reopen the moment the user
//! opts back in: Discord does not repeat a speaking update while an utterance
//! continues, so waiting for the next one would lose the rest of it.

use std::collections::HashMap;

#[derive(Default)]
pub(super) struct OptedOutSpeakers {
    ssrcs: HashMap<u64, u32>,
}

impl OptedOutSpeakers {
    pub(super) fn remember(&mut self, user_id: u64, ssrc: u32) {
        self.ssrcs.insert(user_id, ssrc);
    }

    pub(super) fn forget(&mut self, user_id: u64) -> Option<u32> {
        self.ssrcs.remove(&user_id)
    }

    pub(super) fn user_ids(&self) -> Vec<u64> {
        self.ssrcs.keys().copied().collect()
    }

    /// Speakers from another channel are meaningless after a move.
    pub(super) fn clear(&mut self) {
        self.ssrcs.clear();
    }
}

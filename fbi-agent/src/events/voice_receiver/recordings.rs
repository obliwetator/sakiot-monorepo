//! Owns the SSRC <-> user <-> recording bookkeeping for one voice channel.
//!
//! The recorder tracks the same fact through several maps at once: which writer
//! is open for which SSRC, which SSRC a user currently speaks on, and which
//! SSRCs belong to bots. Keeping those
//! in sync by hand at every call site is how ghost recordings leak (an `active`
//! writer whose user already left, or a `user_ssrcs` entry with no writer).
//!
//! `Recordings` hides those maps and only exposes operations that mutate the
//! related collections together, so the invariants below hold by construction:
//!
//! - `user_ssrcs[uid] == ssrc`  <=>  `active[ssrc]` exists and belongs to `uid`.
//! - `bots.user_ssrcs[uid] == ssrc`  =>  `bots.ssrcs` contains `ssrc`.
//! - `stats.active_user_count == active.len()` (synced on every active mutation).

#[cfg(test)]
use crate::cast::ToI64;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

use super::state::UserRecording;

/// Cross-task snapshot read by the `RecorderHandle` (drop accounting) and by the
/// metrics layer. The actor writes it; other tasks only read.
#[derive(Default)]
pub(super) struct RecorderStats {
    active_user_count: AtomicUsize,
    pub(super) last_voice_packet_time: AtomicI64,
}

impl RecorderStats {
    pub(super) fn active_user_count(&self) -> usize {
        self.active_user_count.load(Ordering::Relaxed)
    }

    pub(super) fn last_voice_packet_time(&self) -> i64 {
        self.last_voice_packet_time.load(Ordering::Relaxed)
    }
}

/// Bot SSRCs we deliberately do not record. Kept as a pair so a bot's `user_id`
/// and its SSRC can never disagree.
#[derive(Default)]
struct Bots {
    ssrcs: HashSet<u32>,
    user_ssrcs: HashMap<u64, u32>,
}

impl Bots {
    fn is_bot(&self, ssrc: u32) -> bool {
        self.ssrcs.contains(&ssrc)
    }

    /// If this user is a known bot, re-point it at `ssrc` and return `true`.
    fn remap(&mut self, user_id: u64, ssrc: u32) -> bool {
        let Some(previous) = self.user_ssrcs.get(&user_id).copied() else {
            return false;
        };
        if previous != ssrc {
            self.ssrcs.remove(&previous);
            self.ssrcs.insert(ssrc);
            self.user_ssrcs.insert(user_id, ssrc);
        }
        true
    }

    fn insert(&mut self, user_id: u64, ssrc: u32) {
        self.ssrcs.insert(ssrc);
        self.user_ssrcs.insert(user_id, ssrc);
    }

    fn remove_user(&mut self, user_id: u64) -> Option<u32> {
        let ssrc = self.user_ssrcs.remove(&user_id)?;
        self.ssrcs.remove(&ssrc);
        Some(ssrc)
    }

    fn clear(&mut self) {
        self.ssrcs.clear();
        self.user_ssrcs.clear();
    }
}

/// Result of trying to re-point an active user's writer at a new SSRC.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemapOutcome {
    /// User has no active writer; caller should open one.
    NotActive,
    /// User already on this SSRC; nothing to do.
    AlreadyActive,
    /// User was tracked but had no writer; stale entry was dropped.
    Stale,
    /// Writer moved from `from` to `to`.
    Remapped { from: u32, to: u32 },
}

pub(super) struct Recordings {
    active: HashMap<u32, UserRecording>,
    user_ssrcs: HashMap<u64, u32>,
    bots: Bots,
    stats: Arc<RecorderStats>,
}

impl Recordings {
    pub(super) fn new(stats: Arc<RecorderStats>) -> Self {
        Self {
            active: HashMap::new(),
            user_ssrcs: HashMap::new(),
            bots: Bots::default(),
            stats,
        }
    }

    fn sync_count(&self) {
        self.stats
            .active_user_count
            .store(self.active.len(), Ordering::Relaxed);
    }

    // --- bots -------------------------------------------------------------

    #[cfg(test)]
    pub(super) fn is_bot_ssrc(&self, ssrc: u32) -> bool {
        self.bots.is_bot(ssrc)
    }

    pub(super) fn remap_bot(&mut self, user_id: u64, ssrc: u32) -> bool {
        self.bots.remap(user_id, ssrc)
    }

    pub(super) fn insert_bot(&mut self, user_id: u64, ssrc: u32) {
        self.bots.insert(user_id, ssrc);
    }

    pub(super) fn remove_bot_user(&mut self, user_id: u64) -> Option<u32> {
        self.bots.remove_user(user_id)
    }

    // --- active users -----------------------------------------------------

    #[cfg(test)]
    pub(super) fn ssrc_for_user(&self, user_id: u64) -> Option<u32> {
        self.user_ssrcs.get(&user_id).copied()
    }

    pub(super) fn has_active_ssrc(&self, ssrc: u32) -> bool {
        self.active.contains_key(&ssrc)
    }

    pub(super) fn has_users(&self) -> bool {
        !self.user_ssrcs.is_empty()
    }

    /// Move a user's open writer onto a new SSRC. See [`RemapOutcome`].
    pub(super) fn remap_active_user(&mut self, user_id: u64, ssrc: u32) -> RemapOutcome {
        let Some(previous_ssrc) = self.user_ssrcs.get(&user_id).copied() else {
            return RemapOutcome::NotActive;
        };
        if previous_ssrc == ssrc {
            return RemapOutcome::AlreadyActive;
        }
        let Some(mut recording) = self.active.remove(&previous_ssrc) else {
            self.user_ssrcs.remove(&user_id);
            self.sync_count();
            return RemapOutcome::Stale;
        };
        recording.ssrc = ssrc;
        self.active.insert(ssrc, recording);
        self.user_ssrcs.insert(user_id, ssrc);
        self.sync_count();
        RemapOutcome::Remapped {
            from: previous_ssrc,
            to: ssrc,
        }
    }

    pub(super) fn insert_active(&mut self, user_id: u64, ssrc: u32, recording: UserRecording) {
        self.active.insert(ssrc, recording);
        self.user_ssrcs.insert(user_id, ssrc);
        self.sync_count();
    }

    pub(super) fn active_get_mut(&mut self, ssrc: u32) -> Option<&mut UserRecording> {
        self.active.get_mut(&ssrc)
    }

    pub(super) fn active_non_bot_ssrcs(&self) -> Vec<u32> {
        self.active
            .keys()
            .copied()
            .filter(|ssrc| !self.bots.is_bot(*ssrc))
            .collect()
    }

    /// Remove an active writer by SSRC, also dropping its `user_ssrcs` entry.
    #[cfg(test)]
    pub(super) fn remove_active_by_ssrc(&mut self, ssrc: u32) -> Option<UserRecording> {
        let recording = self.active.remove(&ssrc);
        if let Some(recording) = &recording {
            self.user_ssrcs.remove(&recording.user_id);
        }
        self.sync_count();
        recording
    }

    /// Remove an active writer by user, also dropping its `user_ssrcs` entry.
    /// Returns `None` if the user was tracked but had no writer.
    pub(super) fn remove_active_by_user(&mut self, user_id: u64) -> Option<UserRecording> {
        let ssrc = self.user_ssrcs.remove(&user_id)?;
        let recording = self.active.remove(&ssrc);
        self.sync_count();
        recording
    }

    pub(super) fn user_ssrc_pairs(&self) -> Vec<(u64, u32)> {
        self.user_ssrcs
            .iter()
            .map(|(&uid, &ssrc)| (uid, ssrc))
            .collect()
    }

    // --- finalize / heartbeat --------------------------------------------

    /// Audio file ids for every active recording — the set the heartbeat must
    /// keep alive in the database.
    pub(super) fn tracked_audio_file_ids(&self) -> Vec<i64> {
        let ids: HashSet<i64> = self
            .active
            .values()
            .map(|recording| recording.audio_file_id)
            .collect();
        ids.into_iter().collect()
    }

    /// Drain every active writer, clearing the user index.
    pub(super) fn take_all_active(&mut self) -> Vec<(u32, UserRecording)> {
        let active = std::mem::take(&mut self.active);
        self.user_ssrcs.clear();
        self.sync_count();
        active.into_iter().collect()
    }

    /// Drop all tracked state. Only safe after active recordings have been
    /// drained and finalized; any leftover writers would be lost.
    pub(super) fn clear(&mut self) {
        self.active.clear();
        self.user_ssrcs.clear();
        self.bots.clear();
        self.sync_count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ogg_opus_writer::OggOpusWriter;

    fn dummy_recording(user_id: u64, ssrc: u32) -> UserRecording {
        let path = std::env::temp_dir().join(format!("fbi-rec-test-{user_id}-{ssrc}.ogg"));
        let file = std::fs::File::create(&path).unwrap();
        let writer = OggOpusWriter::new(std::io::BufWriter::new(file), ssrc, 0).unwrap();
        let _ = std::fs::remove_file(&path);
        UserRecording {
            writer,
            audio_file_id: ssrc.to_i64(),
            recording_session_id: ssrc.to_i64(),
            file_name: format!("rec-{user_id}"),
            start_time: chrono::Utc::now(),
            user_id,
            ssrc,
        }
    }

    fn new_recordings() -> (Recordings, Arc<RecorderStats>) {
        let stats = Arc::new(RecorderStats::default());
        (Recordings::new(stats.clone()), stats)
    }

    #[test]
    fn open_then_remove_by_ssrc_leaves_no_orphan() {
        let (mut rec, stats) = new_recordings();
        rec.insert_active(7, 100, dummy_recording(7, 100));
        assert_eq!(stats.active_user_count(), 1);
        assert!(rec.has_active_ssrc(100));
        assert_eq!(rec.ssrc_for_user(7), Some(100));

        let removed = rec.remove_active_by_ssrc(100);
        assert!(removed.is_some());
        assert_eq!(stats.active_user_count(), 0);
        assert!(!rec.has_active_ssrc(100));
        assert_eq!(
            rec.ssrc_for_user(7),
            None,
            "user index must not outlive writer"
        );
    }

    #[test]
    fn remove_by_user_clears_both_maps() {
        let (mut rec, stats) = new_recordings();
        rec.insert_active(7, 100, dummy_recording(7, 100));
        let removed = rec.remove_active_by_user(7);
        assert!(removed.is_some());
        assert_eq!(stats.active_user_count(), 0);
        assert!(!rec.has_active_ssrc(100));
        assert_eq!(rec.ssrc_for_user(7), None);
    }

    #[test]
    fn remap_moves_writer_and_keeps_count() {
        let (mut rec, stats) = new_recordings();
        rec.insert_active(7, 100, dummy_recording(7, 100));

        assert_eq!(
            rec.remap_active_user(7, 200),
            RemapOutcome::Remapped { from: 100, to: 200 }
        );
        assert_eq!(stats.active_user_count(), 1);
        assert!(!rec.has_active_ssrc(100), "old ssrc must be gone");
        assert!(rec.has_active_ssrc(200));
        assert_eq!(rec.ssrc_for_user(7), Some(200));

        assert_eq!(rec.remap_active_user(7, 200), RemapOutcome::AlreadyActive);
        assert_eq!(rec.remap_active_user(9, 300), RemapOutcome::NotActive);
    }

    #[test]
    fn remap_drops_stale_user_index_without_writer() {
        let (mut rec, _stats) = new_recordings();
        // user indexed but no writer => the "stale" branch must clean it up.
        rec.insert_active(7, 100, dummy_recording(7, 100));
        let _ = rec.remove_active_by_ssrc(100);
        // re-create only the index half to simulate drift, via pause/resume path.
        rec.insert_active(7, 100, dummy_recording(7, 100));
        let _orphan = rec.take_all_active(); // clears active + user_ssrcs together
        assert_eq!(rec.ssrc_for_user(7), None);
    }

    #[test]
    fn bots_remember_remap_and_forget() {
        let (mut rec, _stats) = new_recordings();
        assert!(!rec.remap_bot(42, 100), "unknown bot cannot remap");
        rec.insert_bot(42, 100);
        assert!(rec.is_bot_ssrc(100));

        assert!(rec.remap_bot(42, 101), "known bot remaps");
        assert!(rec.is_bot_ssrc(101));
        assert!(!rec.is_bot_ssrc(100), "old bot ssrc dropped");

        assert_eq!(rec.remove_bot_user(42), Some(101));
        assert!(!rec.is_bot_ssrc(101));
    }
}

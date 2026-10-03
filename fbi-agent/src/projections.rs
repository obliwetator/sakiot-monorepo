//! Keeps the member and voice presence projections (`guild_members`,
//! `voice_presence`) in step with serenity's cache, for the guilds this
//! instance owns. See `database::projections` for the fencing rules.
//!
//! - A guild is claimed only from a complete snapshot: the member list in
//!   GUILD_CREATE when Discord sent all of it (below `large_threshold`), or
//!   the cache after a full chunk set this instance requested.
//! - Chunk requests go out one guild at a time, at most one per
//!   `CHUNK_REQUEST_GAP`, each waiting for its full set or `CHUNK_TIMEOUT`.
//! - A draining instance never claims, but keeps writing the guilds it owns
//!   until its replacement claims them.
//! - Every write, from any event, reads the cache's current state for the
//!   affected user or guild, so writes are order-insensitive.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::{DashMap, DashSet};
use serenity::all::{ChunkGuildFilter, GuildId, ShardMessenger, UserId};
use serenity::cache::Cache;
use serenity::model::event::GuildMembersChunkEvent;
use sqlx::{Pool, Postgres};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{info, warn};

use crate::cast::ToI64;
use crate::database::DbResult;
use crate::database::projections::{self as db, GuildSnapshot, MemberRow, PresenceRow};
use crate::runtime::RuntimeState;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK_REQUEST_GAP: Duration = Duration::from_secs(1);

/// What a refresh found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// Written (claimed, or refreshed as the owner), or nothing to do.
    Done,
    /// The cache does not hold the whole roster: request member chunks.
    NeedsChunks,
}

/// Progress of one chunk request, identified by its nonce.
#[derive(Debug)]
pub(crate) struct ChunkSet {
    guild_id: GuildId,
    count: Option<u32>,
    seen: HashSet<u32>,
}

impl ChunkSet {
    pub(crate) fn new(guild_id: GuildId) -> Self {
        Self {
            guild_id,
            count: None,
            seen: HashSet::new(),
        }
    }

    /// Records one chunk; true once every index of the set arrived. A chunk
    /// for another guild, or an index outside the announced count, is
    /// ignored.
    pub(crate) fn record(&mut self, guild_id: GuildId, index: u32, count: u32) -> bool {
        if guild_id != self.guild_id || index >= count {
            return false;
        }
        let count = *self.count.get_or_insert(count);
        if index < count {
            self.seen.insert(index);
        }
        u32::try_from(self.seen.len()).unwrap_or(u32::MAX) >= count
    }
}

struct PendingChunks {
    set: ChunkSet,
    done: Option<oneshot::Sender<()>>,
}

pub(crate) struct Projections {
    pool: Pool<Postgres>,
    runtime: Arc<RuntimeState>,
    /// Per guild: the generation this instance owns, if any. The async mutex
    /// also serializes this instance's writes for the guild, so a claim and
    /// the generation it records are one step.
    owned: DashMap<GuildId, Arc<Mutex<Option<i64>>>>,
    shards: DashMap<GuildId, ShardMessenger>,
    pending: DashMap<String, PendingChunks>,
    queued: DashSet<GuildId>,
    requests: mpsc::UnboundedSender<GuildId>,
    nonce: AtomicU64,
}

impl Projections {
    pub(crate) fn new(
        pool: Pool<Postgres>,
        runtime: Arc<RuntimeState>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<GuildId>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                pool,
                runtime,
                owned: DashMap::new(),
                shards: DashMap::new(),
                pending: DashMap::new(),
                queued: DashSet::new(),
                requests,
                nonce: AtomicU64::new(0),
            }),
            receiver,
        )
    }

    fn instance_id(&self) -> &str {
        &self.runtime.config().instance_id
    }

    fn slot(&self, guild_id: GuildId) -> Arc<Mutex<Option<i64>>> {
        self.owned.entry(guild_id).or_default().clone()
    }

    /// A guild arrived (GUILD_CREATE): remember its shard for chunk
    /// requests, then claim or refresh it.
    pub(crate) async fn guild_available(
        &self,
        cache: &Cache,
        guild_id: GuildId,
        shard: ShardMessenger,
    ) {
        self.shards.insert(guild_id, shard);
        self.refresh_or_request(cache, guild_id).await;
    }

    /// Refreshes every cached guild: the owner rewrites what it owns (fixing
    /// anything an error skipped), and an active instance claims the rest.
    pub(crate) async fn refresh_all(&self, cache: &Cache) {
        for guild_id in cache.guilds() {
            self.refresh_or_request(cache, guild_id).await;
        }
    }

    async fn refresh_or_request(&self, cache: &Cache, guild_id: GuildId) {
        match self.refresh(cache, guild_id).await {
            Ok(Refresh::Done) => {}
            Ok(Refresh::NeedsChunks) => self.request_chunks(guild_id),
            Err(err) => warn!(
                guild_id = guild_id.get(),
                error = %err,
                "failed to refresh member and presence projections"
            ),
        }
    }

    fn request_chunks(&self, guild_id: GuildId) {
        if self.queued.insert(guild_id) && self.requests.send(guild_id).is_err() {
            self.queued.remove(&guild_id);
        }
    }

    /// Writes the guild's whole projection: as the owner, under the fence;
    /// otherwise, when active and the cache is complete, by claiming it.
    pub(crate) async fn refresh(&self, cache: &Cache, guild_id: GuildId) -> DbResult<Refresh> {
        let slot = self.slot(guild_id);
        let mut owned = slot.lock().await;
        let Some(snapshot) = cache.guild(guild_id).map(|guild| GuildSnapshot::of(&guild)) else {
            return Ok(Refresh::Done);
        };

        if let Some(generation) = *owned {
            let mut transaction = self.pool.begin().await?;
            if db::fence(
                &mut transaction,
                snapshot.guild_id,
                self.instance_id(),
                generation,
            )
            .await?
            {
                db::write_snapshot(&mut transaction, &snapshot).await?;
                transaction.commit().await?;
                return Ok(if snapshot.roster_complete {
                    Refresh::Done
                } else {
                    Refresh::NeedsChunks
                });
            }
            transaction.rollback().await?;
            *owned = None;
        }

        if self.runtime.is_draining() {
            return Ok(Refresh::Done);
        }
        if !snapshot.roster_complete {
            return Ok(Refresh::NeedsChunks);
        }
        let mut transaction = self.pool.begin().await?;
        let generation = db::claim(&mut transaction, snapshot.guild_id, self.instance_id()).await?;
        db::write_snapshot(&mut transaction, &snapshot).await?;
        transaction.commit().await?;
        *owned = Some(generation);
        info!(
            guild_id = guild_id.get(),
            generation,
            members = snapshot.members.len(),
            in_voice = snapshot.presence.len(),
            "claimed member and presence projections"
        );
        Ok(Refresh::Done)
    }

    /// One user's roster row and voice state changed (voice state, member
    /// add or update): write what the cache holds now.
    pub(crate) async fn user_changed(&self, cache: &Cache, guild_id: GuildId, user_id: UserId) {
        if let Err(err) = self.write_user(cache, guild_id, user_id, false).await {
            warn!(
                guild_id = guild_id.get(),
                user_id = user_id.get(),
                error = %err,
                "failed to write member or presence projection"
            );
        }
    }

    /// A member-remove event: the only per-user departure.
    pub(crate) async fn member_left(&self, cache: &Cache, guild_id: GuildId, user_id: UserId) {
        if let Err(err) = self.write_user(cache, guild_id, user_id, true).await {
            warn!(
                guild_id = guild_id.get(),
                user_id = user_id.get(),
                error = %err,
                "failed to remove member from projections"
            );
        }
    }

    async fn write_user(
        &self,
        cache: &Cache,
        guild_id: GuildId,
        user_id: UserId,
        left: bool,
    ) -> DbResult<()> {
        let slot = self.slot(guild_id);
        let mut owned = slot.lock().await;
        let Some(generation) = *owned else {
            return Ok(());
        };
        let mut transaction = self.pool.begin().await?;
        if !db::fence(
            &mut transaction,
            guild_id.to_i64(),
            self.instance_id(),
            generation,
        )
        .await?
        {
            transaction.rollback().await?;
            *owned = None;
            return Ok(());
        }
        if left {
            db::remove_member(&mut transaction, guild_id.to_i64(), user_id.to_i64()).await?;
        } else {
            let (member, presence) = cache
                .guild(guild_id)
                .map(|guild| {
                    (
                        guild.members.get(&user_id).map(MemberRow::from_member),
                        guild
                            .voice_states
                            .get(&user_id)
                            .and_then(PresenceRow::from_voice_state),
                    )
                })
                .unwrap_or((None, None));
            db::write_user(
                &mut transaction,
                guild_id.to_i64(),
                member.as_ref(),
                user_id.to_i64(),
                presence.as_ref(),
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// A member chunk arrived; completes its request once the set is whole.
    pub(crate) fn chunk_received(&self, chunk: &GuildMembersChunkEvent) {
        let Some(nonce) = chunk.nonce.as_deref() else {
            return;
        };
        let complete = match self.pending.get_mut(nonce) {
            Some(mut pending) => {
                pending
                    .set
                    .record(chunk.guild_id, chunk.chunk_index, chunk.chunk_count)
            }
            None => return,
        };
        if complete
            && let Some((_, mut pending)) = self.pending.remove(nonce)
            && let Some(done) = pending.done.take()
        {
            let _ = done.send(());
        }
    }

    /// Requests one guild's members and waits for the full set.
    async fn fetch_roster(&self, guild_id: GuildId) -> bool {
        let Some(shard) = self.shards.get(&guild_id).map(|shard| shard.clone()) else {
            return false;
        };
        let nonce = format!(
            "r{}-{}",
            guild_id.get(),
            self.nonce.fetch_add(1, Ordering::Relaxed)
        );
        let (done, finished) = oneshot::channel();
        self.pending.insert(
            nonce.clone(),
            PendingChunks {
                set: ChunkSet::new(guild_id),
                done: Some(done),
            },
        );
        shard.chunk_guild(
            guild_id,
            None,
            false,
            ChunkGuildFilter::None,
            Some(nonce.clone()),
        );
        let complete = matches!(
            tokio::time::timeout(CHUNK_TIMEOUT, finished).await,
            Ok(Ok(()))
        );
        self.pending.remove(&nonce);
        complete
    }
}

/// Requests member chunks one guild at a time and claims each guild once its
/// roster is complete. A guild that times out is retried by the next periodic
/// refresh.
pub(crate) fn spawn_chunk_worker(
    projections: Arc<Projections>,
    cache: Arc<Cache>,
    mut requests: mpsc::UnboundedReceiver<GuildId>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let guild_id = tokio::select! {
                request = requests.recv() => match request {
                    Some(guild_id) => guild_id,
                    None => return,
                },
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                    continue;
                }
            };
            projections.queued.remove(&guild_id);
            if projections.fetch_roster(guild_id).await {
                if let Ok(Refresh::NeedsChunks) = projections.refresh(&cache, guild_id).await {
                    warn!(
                        guild_id = guild_id.get(),
                        "member chunks arrived but the cached roster is still incomplete"
                    );
                }
            } else {
                warn!(
                    guild_id = guild_id.get(),
                    "member chunk request did not complete; retrying at the next refresh"
                );
            }
            tokio::time::sleep(CHUNK_REQUEST_GAP).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_set_completes_only_with_every_index() {
        let guild = GuildId::new(1);
        let mut set = ChunkSet::new(guild);
        assert!(!set.record(guild, 0, 3));
        assert!(!set.record(guild, 2, 3));
        // A repeated index does not count twice.
        assert!(!set.record(guild, 2, 3));
        // Another guild's chunk, or an index past the count, is ignored.
        assert!(!set.record(GuildId::new(2), 1, 3));
        assert!(!set.record(guild, 5, 3));
        assert!(set.record(guild, 1, 3));
    }

    #[test]
    fn a_single_chunk_set_completes_at_once() {
        let guild = GuildId::new(1);
        assert!(ChunkSet::new(guild).record(guild, 0, 1));
    }
}

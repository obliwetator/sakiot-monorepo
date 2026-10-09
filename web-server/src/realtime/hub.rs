//! Open realtime connections, their subscriptions and cached authorization,
//! and the routing of database events to them.
//!
//! Every HTTP worker runs its own single-threaded runtime, so the hub is
//! shared through `Arc` and plain mutexes that are never held across an
//! `.await`. The listener calls [`Hub::dispatch`] from the main runtime; each
//! socket task drains its own [`Connection`] outbox on its worker.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use sqlx::{Pool, Postgres};
use tokio::sync::Notify;

use super::events::{Event, JobAudience};
use super::metrics::{self, metrics};
use super::protocol::{
    PROTOCOL_VERSION, PresenceSeat, PresenceUpdate, Resource, ResyncReason, ServerMessage,
};
use crate::permissions::{SubscriptionAccess, Viewer, subscription_access};
use crate::presence::Seat;

mod presence;
mod routing;

use presence::*;
use routing::*;

/// Messages a connection may have queued before it is considered too slow:
/// the queue is then replaced by one `resync_required`.
const OUTBOX_CAPACITY: usize = 256;

/// Presence changes to one guild in one batch beyond which its subscribers
/// refetch the list instead: a bot resyncing a large guild rewrites everyone.
const PRESENCE_UPDATES_MAX: usize = 64;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding one of these locks cannot leave the data
    // half-updated in a way later users care about, so poisoning is ignored.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What re-authorizing one subscription changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reauthorized {
    /// A refused scope is now allowed.
    Granted,
    /// An allowed scope was recomputed; it may have lost access.
    Recomputed,
    /// A refused scope is still refused.
    StillRefused,
}

#[derive(Debug, Clone)]
struct Subscription {
    guild_id: i64,
    as_role: Option<i64>,
    /// `None` while the scope is refused (not a member, membership still
    /// loading, a preview the viewer may not make). A refused scope is kept
    /// and re-checked on the guild's permission events, so the viewer is
    /// subscribed as soon as access appears.
    access: Option<SubscriptionAccess>,
    /// The `set_scope` call this subscription came from. A recomputed
    /// authorization is stored only if no newer `set_scope` replaced it.
    generation: u64,
    /// The client applies presence changes (`presence` messages) instead of
    /// refetching on `changed` `presence`.
    presence_updates: bool,
}

#[derive(Default)]
struct Outbox {
    messages: VecDeque<ServerMessage>,
    close: Option<u16>,
}

/// One open socket: who it belongs to, what it subscribed to, and the
/// messages waiting to be written to it.
pub struct Connection {
    pub id: u64,
    pub viewer: Viewer,
    outbox: Mutex<Outbox>,
    notify: Notify,
    subscription: Mutex<Option<Subscription>>,
    generation: AtomicU64,
}

impl Connection {
    fn new(id: u64, viewer: Viewer) -> Self {
        Self {
            id,
            viewer,
            outbox: Mutex::new(Outbox::default()),
            notify: Notify::new(),
            subscription: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    /// Queues a message. A connection that falls `OUTBOX_CAPACITY` messages
    /// behind loses them all and gets one `resync_required` instead.
    pub fn push(&self, message: ServerMessage) {
        {
            let mut outbox = lock(&self.outbox);
            if outbox.close.is_some() {
                return;
            }
            if outbox.messages.len() >= OUTBOX_CAPACITY {
                outbox.messages.clear();
                outbox.messages.push_back(ServerMessage::ResyncRequired {
                    v: PROTOCOL_VERSION,
                    reason: ResyncReason::QueueOverflow,
                    guild_id: None,
                });
                metrics().queue_overflows.add(1, &[]);
                metrics::resync(ResyncReason::QueueOverflow, 1);
            } else {
                outbox.messages.push_back(message);
            }
        }
        self.notify.notify_one();
    }

    /// Asks the socket task to close with `code` once it has written what is
    /// already queued.
    pub fn close(&self, code: u16) {
        lock(&self.outbox).close.get_or_insert(code);
        self.notify.notify_one();
    }

    /// Takes everything queued, and the close code if one was requested.
    pub fn take(&self) -> (Vec<ServerMessage>, Option<u16>) {
        let mut outbox = lock(&self.outbox);
        (outbox.messages.drain(..).collect(), outbox.close)
    }

    /// Resolves after the next `push` or `close`, or immediately if one
    /// happened since the last wait.
    pub async fn wait(&self) {
        self.notify.notified().await;
    }

    fn subscription(&self) -> Option<Subscription> {
        lock(&self.subscription).clone()
    }

    #[cfg(test)]
    fn queued(&self) -> Vec<ServerMessage> {
        self.take().0
    }
}

/// Open connections by what they are subscribed to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionStates {
    /// Subscribed to a guild they may view.
    pub subscribed: usize,
    /// Waiting on a refused scope: not a member (yet), membership still
    /// loading, or a role preview they may not make.
    pub refused: usize,
    /// No scope yet: the page shows no guild.
    pub unscoped: usize,
}

pub struct Hub {
    pool: Pool<Postgres>,
    connections: Mutex<HashMap<u64, Arc<Connection>>>,
    next_id: AtomicU64,
}

impl Hub {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            connections: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn register(&self, viewer: Viewer) -> Arc<Connection> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection::new(id, viewer));
        lock(&self.connections).insert(id, Arc::clone(&connection));
        connection
    }

    pub fn unregister(&self, id: u64) {
        lock(&self.connections).remove(&id);
    }

    pub fn connection_count(&self) -> usize {
        lock(&self.connections).len()
    }

    pub fn connection_states(&self) -> ConnectionStates {
        let mut states = ConnectionStates::default();
        for connection in self.connections() {
            match lock(&connection.subscription).as_ref() {
                Some(Subscription {
                    access: Some(_), ..
                }) => states.subscribed += 1,
                Some(_) => states.refused += 1,
                None => states.unscoped += 1,
            }
        }
        states
    }

    fn connections(&self) -> Vec<Arc<Connection>> {
        lock(&self.connections).values().cloned().collect()
    }

    /// Closes every socket at once (server shutdown). Socket tasks write the
    /// close frame and end; nobody waits for them.
    pub fn close_all(&self, code: u16) {
        for connection in self.connections() {
            connection.close(code);
        }
    }

    /// Replaces the connection's scope. On success the client gets
    /// `subscribed`; when the viewer may not see the guild (or the role
    /// preview is not theirs to make) it gets `access_changed` and no scope.
    pub async fn set_scope(
        &self,
        connection: &Connection,
        guild_id: i64,
        as_role: Option<i64>,
        presence_updates: bool,
    ) {
        let generation = connection.generation.fetch_add(1, Ordering::SeqCst) + 1;
        metrics().authorization_recomputes.add(1, &[]);
        let result = subscription_access(&self.pool, guild_id, connection.viewer, as_role).await;
        if connection.generation.load(Ordering::SeqCst) != generation {
            return; // A newer set_scope superseded this one.
        }
        match result {
            Ok(access) => {
                *lock(&connection.subscription) = Some(Subscription {
                    guild_id,
                    as_role,
                    access: Some(access),
                    generation,
                    presence_updates,
                });
                connection.push(ServerMessage::Subscribed {
                    v: PROTOCOL_VERSION,
                    guild_id: guild_id.to_string(),
                    as_role: as_role.map(|role| role.to_string()),
                });
            }
            Err(error) => {
                tracing::debug!(%error, guild_id, user_id = connection.viewer.user_id, "realtime scope refused");
                *lock(&connection.subscription) = Some(Subscription {
                    guild_id,
                    as_role,
                    access: None,
                    generation,
                    presence_updates,
                });
                connection.push(ServerMessage::AccessChanged {
                    v: PROTOCOL_VERSION,
                    guild_id: guild_id.to_string(),
                });
            }
        }
    }

    /// Recomputes authorization for the subscriptions in `guilds` (all when
    /// `None`). A subscription whose viewer lost access stops receiving
    /// events; a refused one whose viewer gained access starts. Returns each
    /// re-checked connection with its guild and what changed.
    async fn reauthorize(
        &self,
        connections: &[Arc<Connection>],
        guilds: Option<&HashSet<i64>>,
    ) -> Vec<(Arc<Connection>, i64, Reauthorized)> {
        let mut checked = Vec::new();
        for connection in connections {
            let Some(subscription) = connection.subscription() else {
                continue;
            };
            if guilds.is_some_and(|guilds| !guilds.contains(&subscription.guild_id)) {
                continue;
            }
            metrics().authorization_recomputes.add(1, &[]);
            let result = subscription_access(
                &self.pool,
                subscription.guild_id,
                connection.viewer,
                subscription.as_role,
            )
            .await;
            if connection.generation.load(Ordering::SeqCst) != subscription.generation {
                continue; // The client changed scope meanwhile.
            }
            let access = result.ok();
            let outcome = match (subscription.access.is_some(), access.is_some()) {
                (false, true) => Reauthorized::Granted,
                (true, _) => Reauthorized::Recomputed,
                (false, false) => Reauthorized::StillRefused,
            };
            *lock(&connection.subscription) = Some(Subscription {
                access,
                ..subscription.clone()
            });
            checked.push((Arc::clone(connection), subscription.guild_id, outcome));
        }
        checked
    }

    /// Sends `resync_required` to every connection after re-authorizing it,
    /// because whatever was missed may include permission changes.
    pub async fn resync_all(&self, reason: ResyncReason) {
        let connections = self.connections();
        self.reauthorize(&connections, None).await;
        for connection in &connections {
            connection.push(ServerMessage::ResyncRequired {
                v: PROTOCOL_VERSION,
                reason,
                guild_id: None,
            });
        }
        metrics::resync(reason, connections.len() as u64);
    }

    /// Routes one coalesced batch of database events. Permission changes are
    /// applied first, so no event in the batch reaches a viewer on the
    /// strength of access they just lost.
    pub async fn dispatch(&self, events: Vec<Event>) {
        let mut resync_everyone = false;
        let mut permission_guilds = HashSet::new();
        let mut changes = Vec::new();
        for event in events {
            match event {
                Event::Permissions { guild_id: None } => resync_everyone = true,
                Event::Permissions {
                    guild_id: Some(guild_id),
                } => {
                    permission_guilds.insert(guild_id);
                }
                Event::Probe { .. } => {}
                other => changes.push(other),
            }
        }

        let connections = self.connections();
        if resync_everyone {
            self.resync_all(ResyncReason::Permissions).await;
        } else if !permission_guilds.is_empty() {
            for (connection, guild_id, outcome) in self
                .reauthorize(&connections, Some(&permission_guilds))
                .await
            {
                match outcome {
                    // As after `set_scope`: the client reconciles on it.
                    // `access_changed` follows because the viewer's guild
                    // list changed too (a guild they just joined).
                    Reauthorized::Granted => {
                        connection.push(ServerMessage::Subscribed {
                            v: PROTOCOL_VERSION,
                            guild_id: guild_id.to_string(),
                            as_role: connection
                                .subscription()
                                .and_then(|subscription| subscription.as_role)
                                .map(|role| role.to_string()),
                        });
                        connection.push(ServerMessage::AccessChanged {
                            v: PROTOCOL_VERSION,
                            guild_id: guild_id.to_string(),
                        });
                    }
                    Reauthorized::Recomputed => connection.push(ServerMessage::AccessChanged {
                        v: PROTOCOL_VERSION,
                        guild_id: guild_id.to_string(),
                    }),
                    Reauthorized::StillRefused => {}
                }
            }
        }
        if changes.is_empty() {
            return;
        }

        let session_ids: Vec<i64> = changes
            .iter()
            .filter_map(|event| match event {
                Event::Session { session_id, .. } => Some(*session_id),
                Event::Clip { session_id, .. } | Event::Stamp { session_id, .. } => *session_id,
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let journeys = match load_journeys(&self.pool, &session_ids).await {
            Ok(journeys) => Some(journeys),
            Err(error) => {
                // Without journeys nothing can be routed by id; fall back to
                // identifier-free invalidations, which leak nothing.
                tracing::warn!(%error, "realtime journey lookup failed");
                None
            }
        };
        let media_job_ids: Vec<String> = changes
            .iter()
            .filter_map(|event| match event {
                Event::Job {
                    job_id,
                    audience: JobAudience::Viewers,
                    ..
                } => Some(job_id.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let job_viewers = load_job_viewers(&self.pool, &media_job_ids)
            .await
            .unwrap_or_else(|error| {
                // Pages waiting on these jobs still poll slowly meanwhile.
                tracing::warn!(%error, "realtime job viewer lookup failed");
                HashMap::new()
            });
        let presence = PresenceBatch::load(&self.pool, &changes).await;

        for connection in &connections {
            let Some(subscription) = connection.subscription() else {
                continue;
            };
            let mut routed = route(
                &changes,
                &subscription,
                connection.viewer.user_id,
                journeys.as_ref(),
            );
            route_jobs(
                &changes,
                &subscription,
                connection.viewer.user_id,
                &job_viewers,
                &mut routed,
            );
            let updates = route_presence(&presence, &subscription, &mut routed);
            for (resource, ids) in routed {
                connection.push(ServerMessage::Changed {
                    v: PROTOCOL_VERSION,
                    guild_id: subscription.guild_id.to_string(),
                    resource,
                    ids: ids.map(|ids| ids.into_iter().collect()),
                });
            }
            if !updates.is_empty() {
                connection.push(ServerMessage::Presence {
                    v: PROTOCOL_VERSION,
                    guild_id: subscription.guild_id.to_string(),
                    updates,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;

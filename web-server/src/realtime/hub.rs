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
use super::protocol::{PROTOCOL_VERSION, Resource, ResyncReason, ServerMessage};
use crate::permissions::{SubscriptionAccess, Viewer, subscription_access};

/// Messages a connection may have queued before it is considered too slow:
/// the queue is then replaced by one `resync_required`.
const OUTBOX_CAPACITY: usize = 256;

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
    pub async fn set_scope(&self, connection: &Connection, guild_id: i64, as_role: Option<i64>) {
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
            for (resource, ids) in routed {
                connection.push(ServerMessage::Changed {
                    v: PROTOCOL_VERSION,
                    guild_id: subscription.guild_id.to_string(),
                    resource,
                    ids: ids.map(|ids| ids.into_iter().collect()),
                });
            }
        }
    }
}

/// The channels a session touches, as the HTTP listings authorize it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Journey {
    starting_channel_id: i64,
    /// The recording tree's rule: the starting channel plus every fragment's.
    tree: HashSet<i64>,
    /// The clip and stamp rule: every fragment's channel, or the starting
    /// channel while there are no fragments.
    media: HashSet<i64>,
}

/// Journeys of live (not deleted) sessions; missing ids are gone or deleted.
async fn load_journeys(
    pool: &Pool<Postgres>,
    session_ids: &[i64],
) -> Result<HashMap<i64, Journey>, sqlx::Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"SELECT rs.id,
                  rs.starting_channel_id,
                  COALESCE(
                      array_agg(DISTINCT af.channel_id) FILTER (WHERE af.channel_id IS NOT NULL),
                      '{}'
                  ) AS "fragment_channels!"
             FROM recording_sessions rs
             LEFT JOIN audio_files af ON af.recording_session_id = rs.id
            WHERE rs.id = ANY($1) AND rs.deletion_requested_at IS NULL
            GROUP BY rs.id"#,
        session_ids
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let fragments: HashSet<i64> = row.fragment_channels.into_iter().collect();
            let mut tree = fragments.clone();
            tree.insert(row.starting_channel_id);
            let media = if fragments.is_empty() {
                HashSet::from([row.starting_channel_id])
            } else {
                fragments
            };
            (
                row.id,
                Journey {
                    starting_channel_id: row.starting_channel_id,
                    tree,
                    media,
                },
            )
        })
        .collect())
}

/// The users attached to each media job (`media_job_viewers`).
async fn load_job_viewers(
    pool: &Pool<Postgres>,
    job_ids: &[String],
) -> Result<HashMap<String, HashSet<i64>>, sqlx::Error> {
    if job_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        "SELECT job_id, user_id FROM media_job_viewers WHERE job_id = ANY($1)",
        job_ids
    )
    .fetch_all(pool)
    .await?;
    let mut viewers: HashMap<String, HashSet<i64>> = HashMap::new();
    for row in rows {
        viewers.entry(row.job_id).or_default().insert(row.user_id);
    }
    Ok(viewers)
}

/// Per resource: `Some(ids)` to refresh those items, `None` to refresh the
/// whole resource without being told which item changed.
type Routed = BTreeMap<Resource, Option<BTreeSet<String>>>;

fn add_id(routed: &mut Routed, resource: Resource, id: String) {
    if let Some(ids) = routed
        .entry(resource)
        .or_insert_with(|| Some(BTreeSet::new()))
    {
        ids.insert(id);
    }
}

fn add_all(routed: &mut Routed, resource: Resource) {
    routed.insert(resource, None);
}

/// Whether a clip or stamp is listed for the subscription, or `None` when its
/// session is gone and that can no longer be decided.
fn media_visible(
    access: &SubscriptionAccess,
    channel_id: Option<i64>,
    session_id: Option<i64>,
    journeys: &HashMap<i64, Journey>,
) -> Option<bool> {
    let channels = &access.media_channels;
    match session_id {
        Some(session_id) => journeys
            .get(&session_id)
            .map(|journey| journey.media.is_subset(channels)),
        None => Some(channel_id.is_some_and(|channel_id| channels.contains(&channel_id))),
    }
}

/// Which `changed` messages one subscription gets for a batch. Ids go only to
/// viewers the HTTP listing would show the item to. A viewer who may have
/// seen a session before a change hid it gets an identifier-free
/// invalidation instead: earlier visibility is not reconstructed.
fn route(
    changes: &[Event],
    subscription: &Subscription,
    user_id: i64,
    journeys: Option<&HashMap<i64, Journey>>,
) -> Routed {
    let mut routed = Routed::new();
    let Some(access) = subscription.access.as_ref() else {
        return routed;
    };
    for change in changes {
        match change {
            Event::Session {
                guild_id,
                session_id,
            } if *guild_id == subscription.guild_id => {
                let channels = &access.tree_channels;
                match journeys.and_then(|journeys| journeys.get(session_id)) {
                    Some(journey) if journey.tree.is_subset(channels) => {
                        add_id(&mut routed, Resource::Recordings, session_id.to_string());
                    }
                    Some(journey) if !channels.contains(&journey.starting_channel_id) => {
                        // Never listed for this viewer: nothing to refresh.
                    }
                    // Deleted, unknown, or visibility may have changed.
                    _ => add_all(&mut routed, Resource::Recordings),
                }
            }
            Event::Recording { guild_id } if *guild_id == subscription.guild_id => {
                add_all(&mut routed, Resource::Recordings);
            }
            Event::Clip {
                guild_id,
                clip_id,
                channel_id,
                session_id,
            } if *guild_id == subscription.guild_id => {
                match journeys
                    .map(|journeys| media_visible(access, *channel_id, *session_id, journeys))
                {
                    Some(Some(true)) => add_id(&mut routed, Resource::Clips, clip_id.clone()),
                    Some(Some(false)) => {}
                    _ => add_all(&mut routed, Resource::Clips),
                }
            }
            Event::Stamp {
                guild_id,
                stamp_id,
                channel_id,
                session_id,
            } if *guild_id == subscription.guild_id => {
                match journeys
                    .map(|journeys| media_visible(access, *channel_id, *session_id, journeys))
                {
                    Some(Some(true)) => {
                        add_id(&mut routed, Resource::Stamps, stamp_id.to_string());
                    }
                    Some(Some(false)) => {}
                    _ => add_all(&mut routed, Resource::Stamps),
                }
            }
            Event::OptOut {
                guild_id,
                user_id: owner,
            } if *guild_id == subscription.guild_id && *owner == user_id => {
                add_all(&mut routed, Resource::RecordingOptOut);
            }
            Event::Settings { guild_id, resource }
                if *guild_id == subscription.guild_id && access.manager =>
            {
                add_all(&mut routed, *resource);
            }
            Event::Presence { guild_id } if *guild_id == subscription.guild_id => {
                add_all(&mut routed, Resource::Presence);
            }
            Event::Members { guild_id } if *guild_id == subscription.guild_id && access.manager => {
                add_all(&mut routed, Resource::Members);
            }
            _ => {}
        }
    }
    routed
}

/// Adds the batch's job events whose status this subscription's viewer may
/// read, as the job status endpoints decide it. Jobs belong to a guild, so
/// only a subscription to that guild hears about them.
fn route_jobs(
    changes: &[Event],
    subscription: &Subscription,
    user_id: i64,
    job_viewers: &HashMap<String, HashSet<i64>>,
    routed: &mut Routed,
) {
    let Some(access) = subscription.access.as_ref() else {
        return;
    };
    for change in changes {
        let Event::Job {
            guild_id,
            job_id,
            audience,
        } = change
        else {
            continue;
        };
        if *guild_id != subscription.guild_id {
            continue;
        }
        let allowed = match audience {
            JobAudience::Viewers => job_viewers
                .get(job_id)
                .is_some_and(|viewers| viewers.contains(&user_id)),
            JobAudience::Owner(owner) => *owner == user_id,
            JobAudience::Managers => access.manager,
        };
        if allowed {
            add_id(routed, Resource::Jobs, job_id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: i64 = 1;
    const PUBLIC: i64 = 100;
    const PRIVATE: i64 = 200;

    fn subscription(tree: &[i64], media: &[i64], manager: bool) -> Subscription {
        Subscription {
            guild_id: GUILD,
            as_role: None,
            access: Some(SubscriptionAccess {
                tree_channels: tree.iter().copied().collect(),
                media_channels: media.iter().copied().collect(),
                manager,
            }),
            generation: 1,
        }
    }

    fn journey(starting: i64, fragments: &[i64]) -> Journey {
        let fragments: HashSet<i64> = fragments.iter().copied().collect();
        let mut tree = fragments.clone();
        tree.insert(starting);
        Journey {
            starting_channel_id: starting,
            tree,
            media: if fragments.is_empty() {
                HashSet::from([starting])
            } else {
                fragments
            },
        }
    }

    fn ids(values: &[&str]) -> Option<BTreeSet<String>> {
        Some(values.iter().map(|value| value.to_string()).collect())
    }

    #[test]
    fn session_ids_reach_only_viewers_who_can_list_the_session() {
        let journeys = HashMap::from([
            (1, journey(PUBLIC, &[PUBLIC])),
            (2, journey(PUBLIC, &[PUBLIC, PRIVATE])),
            (3, journey(PRIVATE, &[PRIVATE])),
        ]);
        let changes: Vec<Event> = (1..=3)
            .map(|session_id| Event::Session {
                guild_id: GUILD,
                session_id,
            })
            .collect();

        // Sees everything: every id.
        let insider = subscription(&[PUBLIC, PRIVATE], &[PUBLIC, PRIVATE], false);
        assert_eq!(
            route(&changes, &insider, 9, Some(&journeys)),
            Routed::from([(Resource::Recordings, ids(&["1", "2", "3"]))])
        );

        // Session 2 moved into a channel this viewer cannot see, so they may
        // have listed it before: an identifier-free invalidation, and session
        // 3 (never visible to them) is not mentioned at all.
        let outsider = subscription(&[PUBLIC], &[PUBLIC], false);
        assert_eq!(
            route(&changes, &outsider, 9, Some(&journeys)),
            Routed::from([(Resource::Recordings, None)])
        );
        let only_hidden = [changes[2].clone()];
        assert!(route(&only_hidden, &outsider, 9, Some(&journeys)).is_empty());
    }

    #[test]
    fn deleted_sessions_and_failed_lookups_never_send_ids() {
        let change = [Event::Session {
            guild_id: GUILD,
            session_id: 7,
        }];
        let viewer = subscription(&[PUBLIC], &[PUBLIC], false);
        assert_eq!(
            route(&change, &viewer, 9, Some(&HashMap::new())),
            Routed::from([(Resource::Recordings, None)])
        );
        assert_eq!(
            route(&change, &viewer, 9, None),
            Routed::from([(Resource::Recordings, None)])
        );
    }

    #[test]
    fn clips_and_stamps_follow_the_media_rule() {
        let journeys = HashMap::from([(5, journey(PUBLIC, &[PRIVATE]))]);
        let changes = [
            Event::Clip {
                guild_id: GUILD,
                clip_id: "loose".into(),
                channel_id: Some(PUBLIC),
                session_id: None,
            },
            Event::Clip {
                guild_id: GUILD,
                clip_id: "hidden".into(),
                channel_id: Some(PUBLIC),
                session_id: Some(5),
            },
            Event::Stamp {
                guild_id: GUILD,
                stamp_id: 3,
                channel_id: Some(PRIVATE),
                session_id: None,
            },
        ];
        // A role preview whose role can join only the public channel.
        let preview = subscription(&[PUBLIC, PRIVATE], &[PUBLIC], true);
        assert_eq!(
            route(&changes, &preview, 9, Some(&journeys)),
            Routed::from([(Resource::Clips, ids(&["loose"]))])
        );
    }

    #[test]
    fn private_and_admin_events_reach_only_their_audience() {
        let changes = [
            Event::OptOut {
                guild_id: GUILD,
                user_id: 9,
            },
            Event::Settings {
                guild_id: GUILD,
                resource: Resource::Cooldowns,
            },
            Event::Recording { guild_id: 2 },
        ];
        let member = subscription(&[PUBLIC], &[PUBLIC], false);
        assert_eq!(
            route(&changes, &member, 9, Some(&HashMap::new())),
            Routed::from([(Resource::RecordingOptOut, None)])
        );
        assert!(route(&changes, &member, 10, Some(&HashMap::new())).is_empty());
        let manager = subscription(&[PUBLIC], &[PUBLIC], true);
        assert_eq!(
            route(&changes, &manager, 10, Some(&HashMap::new())),
            Routed::from([(Resource::Cooldowns, None)])
        );
    }

    #[test]
    fn presence_reaches_every_viewer_without_ids_and_members_only_managers() {
        let changes = [
            Event::Presence { guild_id: GUILD },
            Event::Members { guild_id: GUILD },
            Event::Presence { guild_id: 2 },
        ];
        // Even a viewer who sees no channel at all refetches its own,
        // filtered view: the event says nothing about where anyone is.
        let nobody = subscription(&[], &[], false);
        assert_eq!(
            route(&changes, &nobody, 9, Some(&HashMap::new())),
            Routed::from([(Resource::Presence, None)])
        );
        let manager = subscription(&[PUBLIC], &[PUBLIC], true);
        assert_eq!(
            route(&changes, &manager, 9, Some(&HashMap::new())),
            Routed::from([(Resource::Presence, None), (Resource::Members, None)])
        );
    }

    #[test]
    fn job_events_reach_only_who_may_read_the_job() {
        let changes = [
            Event::Job {
                guild_id: GUILD,
                job_id: "media".into(),
                audience: JobAudience::Viewers,
            },
            Event::Job {
                guild_id: GUILD,
                job_id: "export".into(),
                audience: JobAudience::Owner(9),
            },
            Event::Job {
                guild_id: GUILD,
                job_id: "deletion".into(),
                audience: JobAudience::Managers,
            },
            Event::Job {
                guild_id: 2,
                job_id: "elsewhere".into(),
                audience: JobAudience::Owner(9),
            },
        ];
        let viewers = HashMap::from([("media".to_string(), HashSet::from([9]))]);
        let jobs = |subscription: &Subscription, user_id| {
            let mut routed = Routed::new();
            route_jobs(&changes, subscription, user_id, &viewers, &mut routed);
            routed
        };

        let member = subscription(&[PUBLIC], &[PUBLIC], false);
        assert_eq!(
            jobs(&member, 9),
            Routed::from([(Resource::Jobs, ids(&["export", "media"]))])
        );
        assert!(jobs(&member, 10).is_empty());
        let manager = subscription(&[PUBLIC], &[PUBLIC], true);
        assert_eq!(
            jobs(&manager, 10),
            Routed::from([(Resource::Jobs, ids(&["deletion"]))])
        );
        // A refused scope hears nothing, not even about the viewer's own jobs.
        let refused = Subscription {
            access: None,
            ..member
        };
        assert!(jobs(&refused, 9).is_empty());
    }

    #[test]
    fn a_slow_connection_gets_one_resync_instead_of_its_backlog() {
        let connection = Connection::new(1, Viewer::discord(9));
        for _ in 0..OUTBOX_CAPACITY + 3 {
            connection.push(ServerMessage::Heartbeat {
                v: PROTOCOL_VERSION,
            });
        }
        // The first CAPACITY fill the queue, the next one replaces them with a
        // resync, and the last two queue behind it.
        let queued = connection.queued();
        assert_eq!(queued.len(), 3);
        assert_eq!(
            queued.first(),
            Some(&ServerMessage::ResyncRequired {
                v: PROTOCOL_VERSION,
                reason: ResyncReason::QueueOverflow,
                guild_id: None,
            })
        );

        connection.close(1012);
        connection.push(ServerMessage::Heartbeat {
            v: PROTOCOL_VERSION,
        });
        assert_eq!(connection.take(), (Vec::new(), Some(1012)));
    }
}

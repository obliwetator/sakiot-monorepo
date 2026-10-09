//! Batching voice presence changes and routing each to the viewers who can
//! see the channel a member left or joined.

use super::*;

/// One member's voice state changed in this batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PresenceMove {
    pub(super) user_id: i64,
    /// Every channel the batch's changes say they were in before.
    pub(super) left: BTreeSet<i64>,
    /// Where the voice-presence list shows them now; `None` when it does not.
    pub(super) seat: Option<Seat>,
}

/// A batch's voice presence changes, resolved to where each member is now.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PresenceBatch {
    /// Guilds whose subscribers refetch the whole list: a change that names
    /// no member, more changes than are worth sending one by one, or a failed
    /// lookup.
    pub(super) refetch: HashSet<i64>,
    /// Per guild, the members whose voice state changed.
    pub(super) moves: HashMap<i64, Vec<PresenceMove>>,
}

impl PresenceBatch {
    pub(super) async fn load(pool: &Pool<Postgres>, changes: &[Event]) -> Self {
        let (mut batch, members) = Self::collect(changes);
        let wanted: Vec<(i64, i64)> = members
            .iter()
            .flat_map(|(guild_id, users)| users.keys().map(|user_id| (*guild_id, *user_id)))
            .collect();
        match crate::presence::seats(pool, &wanted).await {
            Ok(mut seats) => {
                for (guild_id, users) in members {
                    let moves = users
                        .into_iter()
                        .map(|(user_id, left)| PresenceMove {
                            user_id,
                            left,
                            seat: seats.remove(&(guild_id, user_id)),
                        })
                        .collect();
                    batch.moves.insert(guild_id, moves);
                }
            }
            Err(error) => {
                // Refetching leaks nothing and loses nothing.
                tracing::warn!(%error, "realtime presence lookup failed");
                batch.refetch.extend(members.into_keys());
            }
        }
        batch
    }

    /// Sorts the batch's presence events into guilds that refetch and, per
    /// remaining guild, the members to look up with the channels they left.
    pub(super) fn collect(
        changes: &[Event],
    ) -> (Self, BTreeMap<i64, BTreeMap<i64, BTreeSet<i64>>>) {
        let mut batch = Self::default();
        let mut members: BTreeMap<i64, BTreeMap<i64, BTreeSet<i64>>> = BTreeMap::new();
        for change in changes {
            match change {
                Event::Presence {
                    guild_id,
                    member: None,
                } => {
                    batch.refetch.insert(*guild_id);
                }
                Event::Presence {
                    guild_id,
                    member: Some(member),
                } => {
                    members
                        .entry(*guild_id)
                        .or_default()
                        .entry(member.user_id)
                        .or_default()
                        .extend(member.left_channel_id);
                }
                _ => {}
            }
        }
        for (guild_id, users) in &members {
            if users.len() > PRESENCE_UPDATES_MAX {
                batch.refetch.insert(*guild_id);
            }
        }
        members.retain(|guild_id, _| !batch.refetch.contains(guild_id));
        (batch, members)
    }
}

/// The batch's presence changes this subscription may see, where the
/// voice-presence endpoint would list them: a member who is now in a channel
/// the viewer can view, or who left one. A viewer that cannot see either
/// side hears nothing about the member. Clients that asked for updates get
/// them; the others, and every client when the guild must refetch, get one
/// `changed` `presence`.
pub(super) fn route_presence(
    presence: &PresenceBatch,
    subscription: &Subscription,
    routed: &mut Routed,
) -> Vec<PresenceUpdate> {
    let Some(access) = subscription.access.as_ref() else {
        return Vec::new();
    };
    let guild_id = subscription.guild_id;
    if presence.refetch.contains(&guild_id) {
        add_all(routed, Resource::Presence);
        return Vec::new();
    }
    let channels = &access.presence_channels;
    let mut updates = Vec::new();
    for change in presence.moves.get(&guild_id).into_iter().flatten() {
        let seat = change
            .seat
            .as_ref()
            .filter(|seat| channels.contains(&seat.channel_id));
        if seat.is_none() && !change.left.iter().any(|left| channels.contains(left)) {
            continue;
        }
        if !subscription.presence_updates {
            add_all(routed, Resource::Presence);
            return Vec::new();
        }
        updates.push(PresenceUpdate {
            user_id: change.user_id.to_string(),
            channel: seat.map(|seat| PresenceSeat {
                channel_id: seat.channel_id.to_string(),
                channel_name: seat.channel_name.clone(),
                member: seat.member.clone(),
            }),
        });
    }
    updates
}

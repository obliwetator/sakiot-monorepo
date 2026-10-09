use super::*;
use crate::realtime::events::PresenceChange;

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
            presence_channels: tree.iter().copied().collect(),
            manager,
        }),
        generation: 1,
        presence_updates: false,
    }
}

/// A subscription that sees presence in `channels`, and applies presence
/// updates when `updates` is set.
fn presence_viewer(channels: &[i64], updates: bool) -> Subscription {
    let mut viewer = subscription(&[], &[], false);
    if let Some(access) = viewer.access.as_mut() {
        access.presence_channels = channels.iter().copied().collect();
    }
    viewer.presence_updates = updates;
    viewer
}

fn seat(user_id: i64, channel_id: i64) -> Seat {
    Seat {
        channel_id,
        channel_name: format!("channel {channel_id}"),
        member: crate::presence::PresenceMember {
            user_id,
            name: Some(format!("user {user_id}")),
            is_bot: false,
            self_mute: false,
            self_deaf: false,
            server_mute: false,
            server_deaf: false,
            streaming: false,
            video: false,
        },
    }
}

pub(super) fn moved(user_id: i64, left: &[i64], now: Option<i64>) -> PresenceMove {
    PresenceMove {
        user_id,
        left: left.iter().copied().collect(),
        seat: now.map(|channel_id| seat(user_id, channel_id)),
    }
}

fn update(user_id: i64, now: Option<i64>) -> PresenceUpdate {
    PresenceUpdate {
        user_id: user_id.to_string(),
        channel: now.map(|channel_id| {
            let seat = seat(user_id, channel_id);
            PresenceSeat {
                channel_id: channel_id.to_string(),
                channel_name: seat.channel_name,
                member: seat.member,
            }
        }),
    }
}

pub(super) fn journey(starting: i64, fragments: &[i64]) -> Journey {
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
fn members_reach_only_managers() {
    let changes = [
        Event::Members { guild_id: GUILD },
        Event::Members { guild_id: 2 },
    ];
    let viewer = subscription(&[PUBLIC], &[PUBLIC], false);
    assert_eq!(
        route(&changes, &viewer, 9, Some(&HashMap::new())),
        Routed::new()
    );
    let manager = subscription(&[PUBLIC], &[PUBLIC], true);
    assert_eq!(
        route(&changes, &manager, 9, Some(&HashMap::new())),
        Routed::from([(Resource::Members, None)])
    );
}

#[test]
fn presence_without_a_member_refetches_for_every_viewer() {
    let (batch, members) = PresenceBatch::collect(&[
        Event::Presence {
            guild_id: GUILD,
            member: None,
        },
        Event::Presence {
            guild_id: GUILD,
            member: Some(PresenceChange {
                user_id: 7,
                left_channel_id: None,
            }),
        },
    ]);
    assert!(members.is_empty());
    // Even a viewer who sees no channel refetches its own, filtered view:
    // the event says nothing about where anyone is.
    for viewer in [presence_viewer(&[], false), presence_viewer(&[], true)] {
        let mut routed = Routed::new();
        assert!(route_presence(&batch, &viewer, &mut routed).is_empty());
        assert_eq!(routed, Routed::from([(Resource::Presence, None)]));
    }
}

#[test]
fn presence_updates_reach_only_viewers_who_see_either_side() {
    let batch = PresenceBatch {
        refetch: HashSet::new(),
        moves: HashMap::from([
            (
                GUILD,
                vec![
                    moved(7, &[PRIVATE], Some(PUBLIC)),
                    moved(8, &[PRIVATE], None),
                    moved(9, &[], Some(PRIVATE)),
                ],
            ),
            (2, vec![moved(10, &[], Some(PUBLIC))]),
        ]),
    };
    let updates_for = |channels: &[i64]| {
        let mut routed = Routed::new();
        let updates = route_presence(&batch, &presence_viewer(channels, true), &mut routed);
        assert_eq!(routed, Routed::new());
        updates
    };

    assert_eq!(updates_for(&[PUBLIC]), [update(7, Some(PUBLIC))]);
    assert_eq!(
        updates_for(&[PUBLIC, PRIVATE]),
        [
            update(7, Some(PUBLIC)),
            update(8, None),
            update(9, Some(PRIVATE))
        ]
    );
    // Someone moving out of sight is removed, never shown where they went.
    assert_eq!(
        updates_for(&[PRIVATE]),
        [update(7, None), update(8, None), update(9, Some(PRIVATE))]
    );
    assert!(updates_for(&[]).is_empty());
}

#[test]
fn clients_without_presence_updates_refetch_only_what_they_can_see() {
    let batch = PresenceBatch {
        refetch: HashSet::new(),
        moves: HashMap::from([(GUILD, vec![moved(9, &[], Some(PRIVATE))])]),
    };
    let mut routed = Routed::new();
    assert!(route_presence(&batch, &presence_viewer(&[PRIVATE], false), &mut routed).is_empty());
    assert_eq!(routed, Routed::from([(Resource::Presence, None)]));

    let mut routed = Routed::new();
    assert!(route_presence(&batch, &presence_viewer(&[PUBLIC], false), &mut routed).is_empty());
    assert_eq!(routed, Routed::new());
}

#[test]
fn presence_changes_coalesce_per_member_and_large_batches_refetch() {
    let change = |guild_id: i64, user_id: i64, left: Option<i64>| Event::Presence {
        guild_id,
        member: Some(PresenceChange {
            user_id,
            left_channel_id: left,
        }),
    };
    let mut changes = vec![
        change(GUILD, 7, None),
        change(GUILD, 7, Some(PUBLIC)),
        change(GUILD, 7, Some(PRIVATE)),
    ];
    changes.extend((0..=PRESENCE_UPDATES_MAX as i64).map(|user_id| change(2, user_id, None)));
    let (batch, members) = PresenceBatch::collect(&changes);

    assert_eq!(batch.refetch, HashSet::from([2]));
    assert_eq!(
        members,
        BTreeMap::from([(
            GUILD,
            BTreeMap::from([(7, BTreeSet::from([PUBLIC, PRIVATE]))])
        )])
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

//! Deciding which connections an event reaches, and with which ids, from
//! each subscription's cached authorization.

use sakiot_permissions::Capabilities;

use super::*;
use crate::realtime::events::Renamed;

/// The channels a session touches and whose voice it holds, as the HTTP
/// listings authorize it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Journey {
    pub(super) speaker: i64,
    /// Removed by the server or being erased: listed to its speaker alone.
    pub(super) removed: bool,
    pub(super) starting_channel_id: i64,
    /// The recording tree's rule: the starting channel plus every fragment's.
    pub(super) tree: HashSet<i64>,
    /// The stamp rule: every fragment's channel, or the starting channel
    /// while there are no fragments.
    pub(super) media: HashSet<i64>,
}

/// Journeys of sessions, removed ones included; missing ids are gone.
pub(super) async fn load_journeys(
    pool: &Pool<Postgres>,
    session_ids: &[i64],
) -> Result<HashMap<i64, Journey>, sqlx::Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"SELECT rs.id,
                  rs.user_id,
                  rs.starting_channel_id,
                  rs.deletion_requested_at IS NOT NULL AS "removed!",
                  COALESCE(
                      array_agg(DISTINCT af.channel_id) FILTER (WHERE af.channel_id IS NOT NULL),
                      '{}'
                  ) AS "fragment_channels!"
             FROM recording_sessions rs
             LEFT JOIN audio_files af ON af.recording_session_id = rs.id
            WHERE rs.id = ANY($1)
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
                    speaker: row.user_id,
                    removed: row.removed,
                    starting_channel_id: row.starting_channel_id,
                    tree,
                    media,
                },
            )
        })
        .collect())
}

/// The users attached to each media job (`media_job_viewers`).
pub(super) async fn load_job_viewers(
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
pub(super) type Routed = BTreeMap<Resource, Option<BTreeSet<String>>>;

fn add_id(routed: &mut Routed, resource: Resource, id: String) {
    if let Some(ids) = routed
        .entry(resource)
        .or_insert_with(|| Some(BTreeSet::new()))
    {
        ids.insert(id);
    }
}

pub(super) fn add_all(routed: &mut Routed, resource: Resource) {
    routed.insert(resource, None);
}

/// The viewer whose own recordings, clips and stamps the subscription lists
/// wherever they were recorded: none in a role preview, since a role has no
/// recordings of its own.
fn own(subscription: &Subscription, user_id: i64) -> Option<i64> {
    subscription.as_role.is_none().then_some(user_id)
}

/// Whether a clip of `speakers`' voices from `channels` is listed for the
/// subscription: one of only the viewer's own voice always is; anyone
/// else's when every one of its channels is a media channel and none of
/// its other speakers keeps their recordings from the viewer.
fn hears_clip(
    access: &SubscriptionAccess,
    own: Option<i64>,
    speakers: &[i64],
    channels: &[i64],
) -> bool {
    let only_own = own.is_some_and(|user_id| {
        !speakers.is_empty() && speakers.iter().all(|&speaker| speaker == user_id)
    });
    only_own
        || (!channels.is_empty()
            && channels
                .iter()
                .all(|channel| access.media_channels.contains(channel))
            && !speakers
                .iter()
                .any(|speaker| access.private.contains(speaker)))
}

/// Whether a stamp is listed for the subscription, or `None` when its
/// session is gone or removed and that can no longer be decided.
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
            .filter(|journey| !journey.removed)
            .map(|journey| journey.media.is_subset(channels)),
        None => Some(channel_id.is_some_and(|channel_id| channels.contains(&channel_id))),
    }
}

/// Which `changed` messages one subscription gets for a batch. Ids go only to
/// viewers the HTTP listing would show the item to. A viewer who may have
/// seen a session before a change hid it gets an identifier-free
/// invalidation instead: earlier visibility is not reconstructed.
pub(super) fn route(
    changes: &[Event],
    subscription: &Subscription,
    user_id: i64,
    journeys: Option<&HashMap<i64, Journey>>,
) -> Routed {
    let mut routed = Routed::new();
    let Some(access) = subscription.access.as_ref() else {
        return routed;
    };
    let own = own(subscription, user_id);
    for change in changes {
        match change {
            Event::Session {
                guild_id,
                session_id,
            } if *guild_id == subscription.guild_id => {
                let channels = &access.tree_channels;
                match journeys.and_then(|journeys| journeys.get(session_id)) {
                    // The viewer's own, removed or not: its entry says which.
                    Some(journey) if own == Some(journey.speaker) => {
                        add_id(&mut routed, Resource::Recordings, session_id.to_string());
                        if journey.removed {
                            // Its stamps are hidden with it.
                            add_all(&mut routed, Resource::Stamps);
                        }
                    }
                    // Its speaker keeps it from the viewer, who never sees
                    // it unless redacted. Consent changing is a `perm`.
                    Some(journey)
                        if access.private.contains(&journey.speaker) && !access.redacts => {}
                    // Listed, or redacted for the viewer: its entry says
                    // which.
                    Some(journey) if !journey.removed && journey.tree.is_subset(channels) => {
                        add_id(&mut routed, Resource::Recordings, session_id.to_string());
                    }
                    Some(journey) if !channels.contains(&journey.starting_channel_id) => {
                        // Never listed for this viewer: nothing to refresh.
                    }
                    // Removed, unknown, or visibility may have changed.
                    _ => add_all(&mut routed, Resource::Recordings),
                }
            }
            Event::Recording { guild_id } if *guild_id == subscription.guild_id => {
                add_all(&mut routed, Resource::Recordings);
            }
            Event::Clip {
                guild_id,
                clip_id,
                channels,
                speakers,
            } if *guild_id == subscription.guild_id => {
                if hears_clip(access, own, speakers, channels) {
                    add_id(&mut routed, Resource::Clips, clip_id.clone());
                }
            }
            Event::Stamp {
                guild_id,
                stamp_id,
                target_id,
                ..
            } if *guild_id == subscription.guild_id && own.is_some() && *target_id == own => {
                // A stamp on the viewer's own voice is theirs.
                add_id(&mut routed, Resource::Stamps, stamp_id.to_string());
            }
            Event::Stamp {
                guild_id,
                target_id: Some(target_id),
                ..
            } if *guild_id == subscription.guild_id && access.private.contains(target_id) => {
                // On a voice kept from the viewer: never listed.
            }
            Event::Stamp {
                guild_id,
                stamp_id,
                channel_id,
                session_id,
                ..
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
            Event::Privacy {
                guild_id,
                user_id: owner,
            } if *guild_id == subscription.guild_id && *owner == user_id => {
                add_all(&mut routed, Resource::RecordingPrivacy);
            }
            Event::Erasure {
                guild_id,
                speaker_id,
                session_id,
                clip_id,
                ..
            } if *guild_id == subscription.guild_id && own == Some(*speaker_id) => {
                // Only the speaker's listings show their erasures.
                if let Some(session_id) = session_id {
                    add_id(&mut routed, Resource::Recordings, session_id.to_string());
                    add_all(&mut routed, Resource::Stamps);
                }
                if let Some(clip_id) = clip_id {
                    add_id(&mut routed, Resource::Clips, clip_id.clone());
                }
            }
            Event::Erasure {
                guild_id,
                session_id: Some(session_id),
                hid_session: true,
                ..
            } if *guild_id == subscription.guild_id => {
                // The session's own payloads say when it is hidden and shown
                // again, but not that its stamps went and came with it.
                if journeys
                    .and_then(|journeys| journeys.get(session_id))
                    .is_some_and(|journey| {
                        journey.media.is_subset(&access.media_channels)
                            && !access.private.contains(&journey.speaker)
                    })
                {
                    add_all(&mut routed, Resource::Stamps);
                }
            }
            Event::Settings { guild_id, resource }
                if *guild_id == subscription.guild_id
                    && access.holds(settings_audience(*resource)) =>
            {
                add_all(&mut routed, *resource);
            }
            Event::Members { guild_id }
                if *guild_id == subscription.guild_id && access.sees_members() =>
            {
                add_all(&mut routed, Resource::Members);
            }
            Event::Names { guild_id, renamed } if *guild_id == subscription.guild_id => {
                route_names(*renamed, access, &mut routed);
            }
            _ => {}
        }
    }
    routed
}

/// Refreshes whatever shows a renamed guild, role or channel, for viewers who
/// see it there. Recordings show channel ids, not names.
fn route_names(renamed: Renamed, access: &SubscriptionAccess, routed: &mut Routed) {
    match renamed {
        Renamed::Guild => add_all(routed, Resource::Guilds),
        Renamed::Roles => {
            if access.sees_members() {
                add_all(routed, Resource::Members);
            }
            if access.holds(Capabilities::MANAGE_PERMISSIONS) {
                add_all(routed, Resource::PermissionRules);
            }
        }
        Renamed::Channel(channel_id) => {
            if access.presence_channels.contains(&channel_id) {
                add_all(routed, Resource::Presence);
            }
            if access.media_channels.contains(&channel_id) {
                add_all(routed, Resource::Stamps);
            }
            if access.holds(Capabilities::MANAGE_SETTINGS) {
                add_all(routed, Resource::RecordingPolicy);
            }
            if access.holds(Capabilities::MANAGE_PERMISSIONS) {
                add_all(routed, Resource::PermissionRules);
            }
        }
    }
}

/// Who may read a settings resource: the rule editor's holders of
/// `manage_permissions`, keeping everyone, everything else's holders of
/// `manage_settings`.
fn settings_audience(resource: Resource) -> Capabilities {
    match resource {
        Resource::PermissionRules => Capabilities::MANAGE_PERMISSIONS,
        Resource::Keeping => Capabilities::empty(),
        _ => Capabilities::MANAGE_SETTINGS,
    }
}

/// Adds the batch's job events whose status this subscription's viewer may
/// read, as the job status endpoints decide it. Jobs belong to a guild, so
/// only a subscription to that guild hears about them.
pub(super) fn route_jobs(
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
            JobAudience::Managers => access.holds(Capabilities::DELETE_RECORDINGS),
        };
        if allowed {
            add_id(routed, Resource::Jobs, job_id.clone());
        }
    }
}

use serenity::model::id::ChannelId;
use sqlx::{Pool, Postgres};
use tracing::{info, warn};

// Voice state event type IDs match rows seeded in the voice_state_event_types
// migrations.
const EVT_SERVER_MUTE: i32 = 1;
const EVT_SERVER_UNMUTE: i32 = 2;
const EVT_SERVER_DEAFEN: i32 = 3;
const EVT_SERVER_UNDEAFEN: i32 = 4;
const EVT_SELF_MUTE: i32 = 5;
const EVT_SELF_UNMUTE: i32 = 6;
const EVT_SELF_DEAFEN: i32 = 7;
const EVT_SELF_UNDEAFEN: i32 = 8;
const EVT_SUPPRESS_ON: i32 = 9;
const EVT_SUPPRESS_OFF: i32 = 10;
const EVT_STREAM_START: i32 = 11;
const EVT_STREAM_STOP: i32 = 12;
const EVT_VIDEO_ON: i32 = 13;
const EVT_VIDEO_OFF: i32 = 14;
const EVT_CHANNEL_JOIN: i32 = 15;
const EVT_CHANNEL_LEAVE: i32 = 16;
const EVT_CHANNEL_SWITCH: i32 = 17;
pub(in crate::events) const EVT_RECORDING_PAUSE: i32 = 18;
pub(in crate::events) const EVT_RECORDING_RESUME: i32 = 19;
pub(in crate::events) const EVT_USER_RECORDING_PAUSE: i32 = 20;
pub(in crate::events) const EVT_USER_RECORDING_RESUME: i32 = 21;

const VOICE_FLAG_SERVER_MUTE: u8 = 1 << 0;
const VOICE_FLAG_SERVER_DEAF: u8 = 1 << 1;
const VOICE_FLAG_SELF_MUTE: u8 = 1 << 2;
const VOICE_FLAG_SELF_DEAF: u8 = 1 << 3;
const VOICE_FLAG_SUPPRESS: u8 = 1 << 4;
const VOICE_FLAG_VIDEO: u8 = 1 << 5;

pub(in crate::events) async fn insert_voice_event(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: Option<i64>,
    user_id: i64,
    event_type_id: i32,
) {
    if let Err(err) = crate::database::voice_events::insert_voice_state_event(
        pool,
        guild_id,
        channel_id,
        user_id,
        event_type_id,
    )
    .await
    {
        warn!(
            guild_id,
            channel_id, user_id, event_type_id, "failed to insert voice_state_event: {}", err
        );
    }
}

pub(super) async fn insert_voice_connection_event(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: Option<i64>,
    owner_instance_id: Option<&str>,
    event_type: &str,
    reason: Option<&str>,
    details: Option<&str>,
) {
    if let Err(err) = crate::database::voice_events::insert_voice_connection_event(
        pool,
        guild_id,
        channel_id,
        owner_instance_id,
        event_type,
        reason,
        details,
    )
    .await
    {
        warn!(
            guild_id,
            channel_id,
            owner_instance_id,
            event_type,
            reason,
            "failed to insert voice_connection_event: {}",
            err
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChannelTransition {
    Joined(ChannelId),
    Left(ChannelId),
    Switched { from: ChannelId, to: ChannelId },
    Unchanged,
}

pub(super) fn channel_transition(
    old: Option<ChannelId>,
    new: Option<ChannelId>,
) -> ChannelTransition {
    match (old, new) {
        (None, Some(new_ch)) => ChannelTransition::Joined(new_ch),
        (Some(old_ch), None) => ChannelTransition::Left(old_ch),
        (Some(old_ch), Some(new_ch)) if old_ch != new_ch => ChannelTransition::Switched {
            from: old_ch,
            to: new_ch,
        },
        _ => ChannelTransition::Unchanged,
    }
}

struct VoiceFlagEvent {
    label: &'static str,
    enabled_event_type_id: i32,
    disabled_event_type_id: i32,
}

fn voice_state_flags(state: &serenity::model::prelude::VoiceState) -> u8 {
    let mut flags = 0;
    if state.mute {
        flags |= VOICE_FLAG_SERVER_MUTE;
    }
    if state.deaf {
        flags |= VOICE_FLAG_SERVER_DEAF;
    }
    if state.self_mute {
        flags |= VOICE_FLAG_SELF_MUTE;
    }
    if state.self_deaf {
        flags |= VOICE_FLAG_SELF_DEAF;
    }
    if state.suppress {
        flags |= VOICE_FLAG_SUPPRESS;
    }
    if state.self_video {
        flags |= VOICE_FLAG_VIDEO;
    }
    flags
}

fn voice_flag_event(flag: u8) -> Option<VoiceFlagEvent> {
    match flag {
        VOICE_FLAG_SERVER_MUTE => Some(VoiceFlagEvent {
            label: "User server muted changed",
            enabled_event_type_id: EVT_SERVER_MUTE,
            disabled_event_type_id: EVT_SERVER_UNMUTE,
        }),
        VOICE_FLAG_SERVER_DEAF => Some(VoiceFlagEvent {
            label: "User server deafened changed",
            enabled_event_type_id: EVT_SERVER_DEAFEN,
            disabled_event_type_id: EVT_SERVER_UNDEAFEN,
        }),
        VOICE_FLAG_SELF_MUTE => Some(VoiceFlagEvent {
            label: "User self muted changed",
            enabled_event_type_id: EVT_SELF_MUTE,
            disabled_event_type_id: EVT_SELF_UNMUTE,
        }),
        VOICE_FLAG_SELF_DEAF => Some(VoiceFlagEvent {
            label: "User self deafened changed",
            enabled_event_type_id: EVT_SELF_DEAFEN,
            disabled_event_type_id: EVT_SELF_UNDEAFEN,
        }),
        VOICE_FLAG_SUPPRESS => Some(VoiceFlagEvent {
            label: "User suppress status changed",
            enabled_event_type_id: EVT_SUPPRESS_ON,
            disabled_event_type_id: EVT_SUPPRESS_OFF,
        }),
        VOICE_FLAG_VIDEO => Some(VoiceFlagEvent {
            label: "User video status changed",
            enabled_event_type_id: EVT_VIDEO_ON,
            disabled_event_type_id: EVT_VIDEO_OFF,
        }),
        _ => None,
    }
}

async fn record_changed_voice_flag_events(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_id: Option<i64>,
    user_id: i64,
    log_changes: bool,
    old_flags: u8,
    new_flags: u8,
) {
    let mut changed_flags = old_flags ^ new_flags;
    while changed_flags != 0 {
        let flag = 1u8 << changed_flags.trailing_zeros();
        changed_flags &= !flag;

        let Some(event) = voice_flag_event(flag) else {
            continue;
        };
        let old_value = old_flags & flag != 0;
        let new_value = new_flags & flag != 0;

        if log_changes {
            info!("{}: {} -> {}", event.label, old_value, new_value);
        }

        insert_voice_event(
            pool,
            guild_id,
            channel_id,
            user_id,
            if new_value {
                event.enabled_event_type_id
            } else {
                event.disabled_event_type_id
            },
        )
        .await;
    }
}

pub(super) async fn record_voice_events(
    pool: &Pool<Postgres>,
    old: Option<&serenity::model::prelude::VoiceState>,
    new: &serenity::model::prelude::VoiceState,
    log_changes: bool,
) {
    let Some(guild_id) = new.guild_id.map(|g| g.get() as i64) else {
        return;
    };
    let user_id = new.user_id.get() as i64;
    let new_channel = new.channel_id.map(|c| c.get() as i64);

    match channel_transition(old.and_then(|o| o.channel_id), new.channel_id) {
        ChannelTransition::Joined(new_ch) => {
            if log_changes {
                info!("User joined voice channel: {}", new_ch);
            }
            insert_voice_event(
                pool,
                guild_id,
                Some(new_ch.get() as i64),
                user_id,
                EVT_CHANNEL_JOIN,
            )
            .await;
        }
        ChannelTransition::Left(old_ch) => {
            if log_changes {
                info!("User left voice channel: {}", old_ch);
            }
            insert_voice_event(
                pool,
                guild_id,
                Some(old_ch.get() as i64),
                user_id,
                EVT_CHANNEL_LEAVE,
            )
            .await;
        }
        ChannelTransition::Switched { from, to } => {
            if log_changes {
                info!("User switched voice channels: {} -> {}", from, to);
            }
            insert_voice_event(
                pool,
                guild_id,
                Some(to.get() as i64),
                user_id,
                EVT_CHANNEL_SWITCH,
            )
            .await;
        }
        ChannelTransition::Unchanged => {}
    }

    let Some(old) = old else { return };

    record_changed_voice_flag_events(
        pool,
        guild_id,
        new_channel,
        user_id,
        log_changes,
        voice_state_flags(old),
        voice_state_flags(new),
    )
    .await;

    let old_streaming = old.self_stream.unwrap_or(false);
    let new_streaming = new.self_stream.unwrap_or(false);
    if old_streaming != new_streaming {
        if log_changes {
            info!(
                "User stream status changed: {:?} -> {:?}",
                old.self_stream, new.self_stream
            );
        }
        insert_voice_event(
            pool,
            guild_id,
            new_channel,
            user_id,
            if new_streaming {
                EVT_STREAM_START
            } else {
                EVT_STREAM_STOP
            },
        )
        .await;
    }

    if log_changes && old.request_to_speak_timestamp != new.request_to_speak_timestamp {
        info!(
            "User request to speak changed: {:?} -> {:?}",
            old.request_to_speak_timestamp, new.request_to_speak_timestamp
        );
    }
}

#[cfg(test)]
mod tests {
    use serenity::model::id::ChannelId;

    use super::{ChannelTransition, channel_transition};

    #[test]
    fn channel_transition_detects_join_leave_switch_and_unchanged() {
        let a = ChannelId::new(10);
        let b = ChannelId::new(20);

        assert_eq!(
            channel_transition(None, Some(a)),
            ChannelTransition::Joined(a)
        );
        assert_eq!(
            channel_transition(Some(a), None),
            ChannelTransition::Left(a)
        );
        assert_eq!(
            channel_transition(Some(a), Some(b)),
            ChannelTransition::Switched { from: a, to: b }
        );
        assert_eq!(
            channel_transition(Some(a), Some(a)),
            ChannelTransition::Unchanged
        );
        assert_eq!(channel_transition(None, None), ChannelTransition::Unchanged);
    }
}

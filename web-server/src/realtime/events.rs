//! Payloads of the `sakiot_realtime` NOTIFY channel, written by the triggers
//! in `sakiot-db/migrations/20261003010000_realtime_notifications.sql` and the
//! migrations after it.

use serde::Deserialize;

use super::protocol::Resource;

/// The NOTIFY channel the triggers publish on.
pub const CHANNEL: &str = "sakiot_realtime";

/// One committed change, as identifiers only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// A logical recording session or one of its fragments changed.
    Session {
        guild_id: i64,
        session_id: i64,
    },
    /// A file without a logical session changed; it carries no id, so every
    /// subscriber of the guild refreshes its recordings.
    Recording {
        guild_id: i64,
    },
    Clip {
        guild_id: i64,
        clip_id: String,
        channel_id: Option<i64>,
        session_id: Option<i64>,
    },
    Stamp {
        guild_id: i64,
        stamp_id: i64,
        channel_id: Option<i64>,
        session_id: Option<i64>,
    },
    OptOut {
        guild_id: i64,
        user_id: i64,
    },
    Settings {
        guild_id: i64,
        resource: Resource,
    },
    /// Permission or membership data changed. `None` means the guild could
    /// not be resolved, so every subscription must be re-authorized.
    Permissions {
        guild_id: Option<i64>,
    },
    /// The guild's member roster changed (managers' member lists and role
    /// counts).
    Members {
        guild_id: i64,
    },
    /// Someone joined, left or changed state in a voice or stage channel. No
    /// channel or user travels with it: every viewer refetches its own
    /// filtered view.
    Presence {
        guild_id: i64,
    },
    /// A background job's state, stage, progress or error changed.
    Job {
        guild_id: i64,
        job_id: String,
        audience: JobAudience,
    },
    /// This server's own round-trip probe.
    Probe {
        nonce: String,
        sent_at_ms: i64,
    },
}

/// Who may read a job's status, and so hears about its changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobAudience {
    /// A media job (waveforms, mixes, downloads): the users attached to it in
    /// `media_job_viewers`.
    Viewers,
    /// A clip export: the user who started it.
    Owner(i64),
    /// A recording deletion: the guild's managers.
    Managers,
}

#[derive(Deserialize)]
struct Payload {
    v: u8,
    k: String,
    g: Option<String>,
    s: Option<String>,
    c: Option<String>,
    id: Option<String>,
    u: Option<String>,
    r: Option<String>,
    n: Option<String>,
    t: Option<i64>,
}

fn id(value: Option<&String>) -> Option<i64> {
    value.and_then(|value| value.parse().ok())
}

/// Parses a notification payload. Unknown versions, kinds and malformed
/// payloads yield `None`: an older or newer writer must never break delivery.
pub fn parse(payload: &str) -> Option<Event> {
    let payload: Payload = serde_json::from_str(payload).ok()?;
    if payload.v != 1 {
        return None;
    }
    let guild_id = id(payload.g.as_ref());
    Some(match payload.k.as_str() {
        "session" => Event::Session {
            guild_id: guild_id?,
            session_id: id(payload.s.as_ref())?,
        },
        "recording" => Event::Recording {
            guild_id: guild_id?,
        },
        "clip" => Event::Clip {
            guild_id: guild_id?,
            clip_id: payload.id?,
            channel_id: id(payload.c.as_ref()),
            session_id: id(payload.s.as_ref()),
        },
        "stamp" => Event::Stamp {
            guild_id: guild_id?,
            stamp_id: id(payload.id.as_ref())?,
            channel_id: id(payload.c.as_ref()),
            session_id: id(payload.s.as_ref()),
        },
        "opt_out" => Event::OptOut {
            guild_id: guild_id?,
            user_id: id(payload.u.as_ref())?,
        },
        "settings" => Event::Settings {
            guild_id: guild_id?,
            resource: match payload.r.as_deref()? {
                "guild_voice_settings" => Resource::VoiceSettings,
                "guild_recording_policy" => Resource::RecordingPolicy,
                "guild_jam_cooldowns" | "user_jam_cooldown_overrides" => Resource::Cooldowns,
                _ => return None,
            },
        },
        "perm" => Event::Permissions { guild_id },
        "members" => Event::Members {
            guild_id: guild_id?,
        },
        "presence" => Event::Presence {
            guild_id: guild_id?,
        },
        "job" => Event::Job {
            guild_id: guild_id?,
            job_id: payload.id?,
            audience: match payload.r.as_deref()? {
                "media" => JobAudience::Viewers,
                "composition" => JobAudience::Owner(id(payload.u.as_ref())?),
                "deletion" => JobAudience::Managers,
                _ => return None,
            },
        },
        "probe" => Event::Probe {
            nonce: payload.n?,
            sent_at_ms: payload.t?,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_trigger_payload() {
        assert_eq!(
            parse(r#"{"v":1,"k":"session","g":"1","s":"2","c":"3"}"#),
            Some(Event::Session {
                guild_id: 1,
                session_id: 2
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"recording","g":"1"}"#),
            Some(Event::Recording { guild_id: 1 })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"clip","g":"1","id":"abc","c":"3"}"#),
            Some(Event::Clip {
                guild_id: 1,
                clip_id: "abc".into(),
                channel_id: Some(3),
                session_id: None,
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"stamp","g":"1","id":"9","s":"2"}"#),
            Some(Event::Stamp {
                guild_id: 1,
                stamp_id: 9,
                channel_id: None,
                session_id: Some(2),
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"opt_out","g":"1","u":"7"}"#),
            Some(Event::OptOut {
                guild_id: 1,
                user_id: 7
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"settings","g":"1","r":"user_jam_cooldown_overrides"}"#),
            Some(Event::Settings {
                guild_id: 1,
                resource: Resource::Cooldowns
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"perm","g":"1"}"#),
            Some(Event::Permissions { guild_id: Some(1) })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"perm"}"#),
            Some(Event::Permissions { guild_id: None })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"members","g":"1"}"#),
            Some(Event::Members { guild_id: 1 })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"presence","g":"1"}"#),
            Some(Event::Presence { guild_id: 1 })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"job","r":"media","g":"1","id":"j1"}"#),
            Some(Event::Job {
                guild_id: 1,
                job_id: "j1".into(),
                audience: JobAudience::Viewers,
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"job","r":"composition","g":"1","id":"j2","u":"7"}"#),
            Some(Event::Job {
                guild_id: 1,
                job_id: "j2".into(),
                audience: JobAudience::Owner(7),
            })
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"job","r":"deletion","g":"1","id":"j3"}"#),
            Some(Event::Job {
                guild_id: 1,
                job_id: "j3".into(),
                audience: JobAudience::Managers,
            })
        );
    }

    #[test]
    fn ignores_what_it_does_not_understand() {
        assert_eq!(parse(r#"{"v":2,"k":"session","g":"1","s":"2"}"#), None);
        assert_eq!(parse(r#"{"v":1,"k":"future","g":"1"}"#), None);
        assert_eq!(parse(r#"{"v":1,"k":"session","g":"x","s":"2"}"#), None);
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"v":1,"k":"presence"}"#), None);
        // An export's owner is required; an unknown job table is skipped.
        assert_eq!(
            parse(r#"{"v":1,"k":"job","r":"composition","g":"1","id":"j"}"#),
            None
        );
        assert_eq!(
            parse(r#"{"v":1,"k":"job","r":"future","g":"1","id":"j"}"#),
            None
        );
    }
}

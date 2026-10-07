//! Messages exchanged over `/api/realtime`. Every message carries the
//! protocol version `v`; Discord ids are strings so JavaScript never rounds
//! them. Events are refresh signals: clients re-read data through the
//! authorized HTTP endpoints. The one exception is voice presence, which
//! clients that ask for it (`presence_updates`) receive as changes to apply.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The only protocol version this server speaks. A client message with any
/// other version is closed with [`CLOSE_UNSUPPORTED_VERSION`].
pub const PROTOCOL_VERSION: u8 = 1;

/// The access token expired; reconnect with a fresh one.
pub const CLOSE_TOKEN_EXPIRED: u16 = 4001;
/// The client spoke an unsupported protocol version: stop reconnecting and
/// prompt for a reload.
pub const CLOSE_UNSUPPORTED_VERSION: u16 = 4400;
/// The server is restarting; reconnect with jitter.
pub const CLOSE_SERVICE_RESTART: u16 = 1012;

/// What a `changed` event says needs refreshing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// The recording tree and what derives from it (live stems, session
    /// manifests). `ids` are recording session ids; without `ids`, refresh the
    /// guild's whole tree, clips and stamps, because visibility may have
    /// changed.
    Recordings,
    /// Clip list; `ids` are clip ids.
    Clips,
    /// Stamp list; `ids` are stamp ids.
    Stamps,
    /// The viewer's own recording opt-out.
    RecordingOptOut,
    /// Guild voice settings (managers only).
    VoiceSettings,
    /// Guild recording policy (managers only).
    RecordingPolicy,
    /// Guild and per-user Jam cooldowns (managers only).
    Cooldowns,
    /// Who is in which voice or stage channel. Never carries `ids`. Clients
    /// that set `presence_updates` get it only when a change cannot be sent
    /// as `presence` updates.
    Presence,
    /// The guild's member roster: member search and role member counts
    /// (managers only).
    Members,
    /// Background jobs the viewer may be waiting on: media builds (waveforms,
    /// mixes, downloads), clip exports and recording deletions. `ids` are job
    /// ids, sent only to whoever may read that job's status.
    Jobs,
}

/// Why the client must refetch everything it has subscribed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    /// The server's database listener lost its connection and may have missed
    /// events.
    ListenerReconnected,
    /// This connection's queue overflowed and events were dropped.
    QueueOverflow,
    /// Permissions changed in a way the server could not attribute to a guild.
    Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// First message on every connection.
    Ready {
        v: u8,
        #[schema(example = "146638124288704513")]
        user_id: String,
        /// Unix milliseconds.
        server_time: i64,
        /// Unix milliseconds; the server closes with 4001 at this time.
        token_expires_at: i64,
    },
    /// The scope requested by `set_scope` is active. Reconcile subscribed data
    /// after this message.
    Subscribed {
        v: u8,
        guild_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        as_role: Option<String>,
    },
    Changed {
        v: u8,
        guild_id: String,
        resource: Resource,
        #[serde(skip_serializing_if = "Option::is_none")]
        ids: Option<Vec<String>>,
    },
    /// The viewer's access to the guild changed, or `set_scope` was refused:
    /// refetch, and drop whatever is no longer allowed.
    AccessChanged { v: u8, guild_id: String },
    ResyncRequired {
        v: u8,
        reason: ResyncReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        guild_id: Option<String>,
    },
    /// Members who joined, left, moved or changed state in the voice and
    /// stage channels the viewer can view, in order. Sent instead of
    /// `changed` `presence` to clients that set `presence_updates`, while the
    /// guild's presence is known. Apply them to the `VoicePresence` list.
    Presence {
        v: u8,
        guild_id: String,
        updates: Vec<PresenceUpdate>,
    },
    /// Sent every 20 s; drives the client's staleness timer.
    Heartbeat { v: u8 },
}

/// Where one member is in the viewer's voice-presence list after a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PresenceUpdate {
    #[schema(example = "146638124288704513")]
    pub user_id: String,
    /// The member's channel and state. Absent when the member is no longer in
    /// a channel the viewer can view: remove them from the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<PresenceSeat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PresenceSeat {
    #[schema(example = "146638124288704513")]
    pub channel_id: String,
    /// The channel's name, as `PresenceChannel.name`.
    pub channel_name: String,
    pub member: crate::presence::PresenceMember,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to one guild, optionally as a manager previewing a role.
    /// Replaces any previous scope.
    SetScope {
        v: u8,
        guild_id: String,
        #[serde(default)]
        as_role: Option<String>,
        /// Receive voice presence changes as `presence` messages instead of
        /// `changed` signals to refetch. Servers without them ignore this
        /// and keep sending `changed`.
        #[serde(default)]
        presence_updates: bool,
    },
    Heartbeat {
        v: u8,
    },
}

impl ClientMessage {
    pub fn version(&self) -> u8 {
        match self {
            Self::SetScope { v, .. } | Self::Heartbeat { v } => *v,
        }
    }
}

/// Reads a client message, telling an unsupported version apart from a
/// malformed one: any JSON object whose `v` is not ours is a version problem.
pub fn parse_client_message(text: &str) -> Result<ClientMessage, ParseError> {
    #[derive(Deserialize)]
    struct Versioned {
        v: Option<u64>,
    }
    let versioned: Versioned = serde_json::from_str(text).map_err(|_| ParseError::Malformed)?;
    if versioned.v != Some(u64::from(PROTOCOL_VERSION)) {
        return Err(ParseError::UnsupportedVersion);
    }
    let message: ClientMessage = serde_json::from_str(text).map_err(|_| ParseError::Malformed)?;
    if message.version() == PROTOCOL_VERSION {
        Ok(message)
    } else {
        Err(ParseError::UnsupportedVersion)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    UnsupportedVersion,
    Malformed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_messages_keep_ids_as_strings() -> Result<(), serde_json::Error> {
        let message = ServerMessage::Changed {
            v: PROTOCOL_VERSION,
            guild_id: "146638124288704513".into(),
            resource: Resource::Recordings,
            ids: Some(vec!["42".into()]),
        };
        assert_eq!(
            serde_json::to_value(&message)?,
            serde_json::json!({
                "type": "changed", "v": 1, "guild_id": "146638124288704513",
                "resource": "recordings", "ids": ["42"],
            })
        );
        let invalidation = ServerMessage::Changed {
            v: PROTOCOL_VERSION,
            guild_id: "1".into(),
            resource: Resource::Recordings,
            ids: None,
        };
        assert!(serde_json::to_value(&invalidation)?.get("ids").is_none());
        Ok(())
    }

    #[test]
    fn presence_updates_say_where_each_member_is_now() -> Result<(), serde_json::Error> {
        let message = ServerMessage::Presence {
            v: PROTOCOL_VERSION,
            guild_id: "1".into(),
            updates: vec![
                PresenceUpdate {
                    user_id: "7".into(),
                    channel: Some(PresenceSeat {
                        channel_id: "3".into(),
                        channel_name: "General".into(),
                        member: crate::presence::PresenceMember {
                            user_id: 7,
                            name: Some("Ann".into()),
                            is_bot: false,
                            self_mute: true,
                            self_deaf: false,
                            server_mute: false,
                            server_deaf: false,
                            streaming: false,
                            video: false,
                        },
                    }),
                },
                PresenceUpdate {
                    user_id: "8".into(),
                    channel: None,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&message)?,
            serde_json::json!({
                "type": "presence", "v": 1, "guild_id": "1",
                "updates": [
                    {
                        "user_id": "7",
                        "channel": {
                            "channel_id": "3", "channel_name": "General",
                            "member": {
                                "user_id": "7", "name": "Ann", "is_bot": false,
                                "self_mute": true, "self_deaf": false,
                                "server_mute": false, "server_deaf": false,
                                "streaming": false, "video": false,
                            },
                        },
                    },
                    { "user_id": "8" },
                ],
            })
        );
        Ok(())
    }

    #[test]
    fn client_messages_are_versioned() {
        assert_eq!(
            parse_client_message(r#"{"type":"set_scope","v":1,"guild_id":"5"}"#),
            Ok(ClientMessage::SetScope {
                v: 1,
                guild_id: "5".into(),
                as_role: None,
                presence_updates: false,
            })
        );
        assert_eq!(
            parse_client_message(
                r#"{"type":"set_scope","v":1,"guild_id":"5","presence_updates":true}"#
            ),
            Ok(ClientMessage::SetScope {
                v: 1,
                guild_id: "5".into(),
                as_role: None,
                presence_updates: true,
            })
        );
        assert_eq!(
            parse_client_message(r#"{"type":"set_scope","v":2,"guild_id":"5"}"#),
            Err(ParseError::UnsupportedVersion)
        );
        assert_eq!(
            parse_client_message(r#"{"type":"heartbeat"}"#),
            Err(ParseError::UnsupportedVersion)
        );
        assert_eq!(
            parse_client_message(r#"{"type":"nonsense","v":1}"#),
            Err(ParseError::Malformed)
        );
        assert_eq!(parse_client_message("not json"), Err(ParseError::Malformed));
    }
}

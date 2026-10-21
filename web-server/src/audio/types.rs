use std::collections::HashMap;

#[derive(serde::Serialize, Debug, utoipa::ToSchema)]
pub struct Directories {
    pub year: i32,
    pub months: Option<Months>,
}

#[derive(serde::Serialize, Debug, utoipa::ToSchema)]
pub struct Channels {
    pub channel_id: String,
    pub dirs: Vec<Directories>,
}

#[derive(serde::Serialize, Debug, utoipa::ToSchema)]
pub struct File {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_journey: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Role-preview annotation only: "can-listen" | "visible-only" | "hidden".
    /// Absent in normal listings, which are filtered to what the viewer can
    /// actually play.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    /// The session's speaker keeps their recordings private, so only who
    /// spoke, where and when are shown: no audio, waveform, live stream,
    /// mix track or stamps. Listed only to holders of `delete_recordings`,
    /// who may remove it, and only where they could otherwise listen.
    pub redacted: bool,
    /// Only on the viewer's own sessions that everyone else no longer sees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removal: Option<Removal>,
    #[serde(skip)]
    #[schema(ignore)]
    pub start_ts_ms: Option<i64>,
}

/// Why one of the viewer's own sessions is hidden from everyone else.
#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
pub struct Removal {
    /// Their erasure of it. Without one, a manager or retention removed it
    /// ("removed by the server"), and erasing it is all they can do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erasure: Option<crate::recording_deletion::ErasureSummary>,
    /// Whether they can still play it: only while erasing a session that
    /// was visible when they asked, and that the server has not removed
    /// since.
    pub playable: bool,
}

pub type Months = HashMap<i32, Option<Vec<File>>>;

#[derive(serde::Deserialize, Debug, utoipa::ToSchema)]
pub struct StartEnd {
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub name: Option<String>,
    pub silence_free: Option<bool>,
}

#[derive(Debug)]
pub struct WaveformProgressContainer(pub tokio::sync::RwLock<HashMap<String, i16>>);

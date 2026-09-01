//! Cache identity, persisted generation settings, and status responses.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{Datelike, Utc};
use sakiot_paths::SessionKey;
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

use super::super::super::live::mark_cache_access;
use super::super::super::paths::recording_path;
use super::super::{AudioFragment, SessionAccess};
use super::occupancy::MixWindow;
use super::{
    ChannelMixGenerationSettings, ChannelMixParticipantSettings, ChannelMixReason,
    ChannelMixResponse, ChannelMixScope, ChannelMixStatus, ChannelMixTrack, MIX_FINGERPRINT,
    MIX_OUTPUT, MIX_SETTINGS, MixContributor, MixPlan, SessionMixContainer,
};

pub(super) fn mix_cache_dir(access: &SessionAccess, anchor_fragments: &[AudioFragment]) -> PathBuf {
    let (channel_id, year, month) = anchor_fragments
        .first()
        .map(|fragment| (fragment.channel_id, fragment.year, fragment.month as u32))
        .unwrap_or_else(|| {
            let date = chrono::DateTime::<Utc>::from_timestamp_millis(access.started_at_ms)
                .unwrap_or_else(Utc::now);
            (access.starting_channel_id, date.year(), date.month())
        });
    SessionKey::new(
        access.guild_id,
        channel_id,
        year,
        month,
        access.started_at_ms,
    )
    .mix_dir(&recording_path())
}

pub(super) fn default_generation_settings(
    tracks: &[ChannelMixTrack],
) -> ChannelMixGenerationSettings {
    let mut participants = tracks
        .iter()
        .map(|track| ChannelMixParticipantSettings {
            user_id: track.user_id.clone(),
            gain_db: 0.0,
            muted: false,
        })
        .collect::<Vec<_>>();
    participants.sort_by_key(|participant| participant.user_id.parse::<i64>().unwrap_or(i64::MAX));
    ChannelMixGenerationSettings {
        participants,
        source_fingerprint: None,
    }
}

pub(super) fn canonical_generation_settings(
    plan: &MixPlan,
    requested: Vec<ChannelMixParticipantSettings>,
) -> Result<ChannelMixGenerationSettings, AppError> {
    let known = plan
        .tracks
        .iter()
        .filter_map(|track| track.user_id.parse::<i64>().ok())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for participant in &requested {
        let user_id = participant.user_id.parse::<i64>().map_err(|_| {
            AppError::BadRequest(format!(
                "Unknown channel mix participant {}",
                participant.user_id
            ))
        })?;
        if !known.contains(&user_id) {
            return Err(AppError::BadRequest(format!(
                "Unknown channel mix participant {}",
                participant.user_id
            )));
        }
        if !seen.insert(user_id) {
            return Err(AppError::BadRequest(format!(
                "Duplicate channel mix participant {}",
                participant.user_id
            )));
        }
        if !participant.gain_db.is_finite() || !(-60.0..=12.0).contains(&participant.gain_db) {
            return Err(AppError::BadRequest(format!(
                "Channel mix gain for {} must be between -60 dB and +12 dB",
                participant.user_id
            )));
        }
    }

    let mut settings = default_generation_settings(&plan.tracks);
    let mut values = requested
        .into_iter()
        .map(|participant| {
            let user_id = participant.user_id.parse::<i64>().unwrap_or_default();
            (
                user_id,
                ChannelMixParticipantSettings {
                    user_id: user_id.to_string(),
                    gain_db: if participant.gain_db == 0.0 {
                        0.0
                    } else {
                        participant.gain_db
                    },
                    muted: participant.muted,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    for participant in &mut settings.participants {
        let user_id = participant.user_id.parse::<i64>().unwrap_or_default();
        if let Some(value) = values.remove(&user_id) {
            *participant = value;
        }
    }
    if !settings.participants.is_empty()
        && settings
            .participants
            .iter()
            .all(|participant| participant.muted)
    {
        return Err(AppError::BadRequest(
            "At least one channel mix participant must be unmuted".into(),
        ));
    }
    Ok(settings)
}

pub(super) fn mix_source_fingerprint(
    access: &SessionAccess,
    scope: ChannelMixScope,
    timeline_start_ms: i64,
    duration_ms: i64,
    windows: &[MixWindow],
    anchors: &[AudioFragment],
    contributors: &[MixContributor],
) -> String {
    let mut parts = vec![format!(
        "v4;scope={};session={};timeline_start={};duration={}",
        scope.as_str(),
        access.session_id,
        timeline_start_ms,
        duration_ms
    )];
    for window in windows {
        parts.push(format!(
            "window:{}:{}:{}",
            window.channel_id, window.start_ms, window.end_ms
        ));
    }
    for fragment in anchors {
        parts.push(format!(
            "anchor:{}:{}:{}:{}:{}:{}:{}:{}",
            fragment.id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            fragment.file_name,
            fragment.year,
            fragment.month,
            fragment.live
        ));
    }
    for contributor in contributors {
        let fragment = &contributor.fragment;
        parts.push(format!(
            "source:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            fragment.id,
            contributor.session_id.unwrap_or(0),
            contributor.user_id,
            fragment.channel_id,
            fragment.start_ms,
            fragment.end_ms.unwrap_or(0),
            contributor.state,
            fragment.file_name,
            fragment.year,
            fragment.month,
            fragment.live
        ));
    }
    parts.join("\n")
}

pub(super) fn mix_fingerprint(
    source_fingerprint: &str,
    settings: &ChannelMixGenerationSettings,
) -> String {
    let settings = serde_json::to_string(&settings.participants).unwrap_or_default();
    format!("{source_fingerprint}\nsettings:{settings}")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct MixCacheMetadata {
    pub(super) source_fingerprint: String,
    pub(super) fingerprint: String,
    pub(super) settings: ChannelMixGenerationSettings,
}

fn response_settings(
    settings: &ChannelMixGenerationSettings,
    source_fingerprint: &str,
) -> ChannelMixGenerationSettings {
    let mut settings = settings.clone();
    settings.source_fingerprint = Some(source_fingerprint.to_owned());
    settings
}

async fn read_cache_metadata(plan: &MixPlan) -> Option<MixCacheMetadata> {
    let output = plan.cache_dir.join(MIX_OUTPUT);
    let fingerprint = plan.cache_dir.join(MIX_FINGERPRINT);
    let settings = plan.cache_dir.join(MIX_SETTINGS);
    if !tokio::fs::try_exists(&output).await.unwrap_or(false)
        || !tokio::fs::try_exists(&fingerprint).await.unwrap_or(false)
        || !tokio::fs::try_exists(&settings).await.unwrap_or(false)
    {
        return None;
    }
    let fingerprint_value = tokio::fs::read_to_string(fingerprint).await.ok()?;
    let metadata = tokio::fs::read_to_string(settings)
        .await
        .ok()
        .and_then(|value| serde_json::from_str::<MixCacheMetadata>(&value).ok())?;
    (metadata.fingerprint == fingerprint_value).then_some(metadata)
}

pub(super) async fn cache_has_current_source(plan: &MixPlan) -> bool {
    read_cache_metadata(plan)
        .await
        .is_some_and(|metadata| metadata.source_fingerprint == plan.source_fingerprint)
}

pub(super) async fn cache_is_valid(plan: &MixPlan) -> bool {
    read_cache_metadata(plan).await.is_some_and(|metadata| {
        metadata.source_fingerprint == plan.source_fingerprint
            && metadata.fingerprint == plan.fingerprint
    })
}

pub(super) async fn mix_response(
    plan: &MixPlan,
    access: &SessionAccess,
    container: &SessionMixContainer,
    include_existing_artifact: bool,
) -> Result<ChannelMixResponse, AppError> {
    if tokio::fs::try_exists(&plan.cache_dir)
        .await
        .unwrap_or(false)
    {
        mark_cache_access(&plan.cache_dir).await;
    }

    let common = |status,
                  reason,
                  progress,
                  media_url,
                  generation_settings: Option<ChannelMixGenerationSettings>| {
        ChannelMixResponse {
            scope: plan.scope,
            status,
            reason,
            progress,
            duration_ms: plan.duration_ms,
            participants: plan.participants.clone(),
            source_count: plan.source_count(),
            media_url,
            can_generate: plan.can_generate(access),
            tracks: plan.tracks.clone(),
            generation_settings,
        }
    };
    if let Some(reason) = plan.blocking_reason(access) {
        return Ok(common(
            ChannelMixStatus::Waiting,
            Some(reason),
            0,
            None,
            None,
        ));
    }
    if plan.sources.is_empty() {
        return Ok(common(
            ChannelMixStatus::Unavailable,
            Some(ChannelMixReason {
                code: "no_overlap".into(),
                message: match plan.scope {
                    ChannelMixScope::AllRecordings => {
                        "No recordings were found while the bot was connected to this channel."
                            .into()
                    }
                    ChannelMixScope::SelectedSession => {
                        "No other recorded participant overlaps the selected session timeline."
                            .into()
                    }
                },
            }),
            0,
            None,
            None,
        ));
    }
    if let Some(job) = container.job(plan.session_id, plan.scope).await {
        let job = job.lock().await;
        if job.source_fingerprint != plan.source_fingerprint {
            return Ok(common(ChannelMixStatus::Idle, None, 0, None, None));
        }
        let settings = Some(response_settings(&job.settings, &job.source_fingerprint));
        if let Some(error) = &job.failed {
            return Ok(common(
                ChannelMixStatus::Failed,
                Some(ChannelMixReason {
                    code: "render_failed".into(),
                    message: error.clone(),
                }),
                0,
                None,
                settings,
            ));
        }
        return Ok(common(
            ChannelMixStatus::Processing,
            None,
            job.progress,
            None,
            settings,
        ));
    }
    if cache_is_valid(plan).await {
        return Ok(common(
            ChannelMixStatus::Ready,
            None,
            100,
            Some(format!(
                "/api/audio/sessions/{}/channel-mix/media?scope={}",
                plan.session_id,
                plan.scope.as_str()
            )),
            Some(response_settings(&plan.settings, &plan.source_fingerprint)),
        ));
    }
    if include_existing_artifact
        && let Some(metadata) = read_cache_metadata(plan).await
        && metadata.source_fingerprint == plan.source_fingerprint
    {
        return Ok(common(
            ChannelMixStatus::Ready,
            None,
            100,
            Some(format!(
                "/api/audio/sessions/{}/channel-mix/media?scope={}",
                plan.session_id,
                plan.scope.as_str()
            )),
            Some(response_settings(
                &metadata.settings,
                &metadata.source_fingerprint,
            )),
        ));
    }
    Ok(common(ChannelMixStatus::Idle, None, 0, None, None))
}

pub(super) fn anchor_wait_reason(access: &SessionAccess) -> Option<ChannelMixReason> {
    match access.state.as_str() {
        "finalized" => None,
        "active" => Some(ChannelMixReason {
            code: "active_anchor".into(),
            message: "The anchor recording is still active.".into(),
        }),
        "pending" => Some(ChannelMixReason {
            code: "pending_anchor".into(),
            message: "The anchor recording is paused and awaiting finalization.".into(),
        }),
        _ => Some(ChannelMixReason {
            code: "unfinalized_anchor".into(),
            message: "The anchor recording is not finalized yet.".into(),
        }),
    }
}

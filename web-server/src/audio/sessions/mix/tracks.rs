//! The per-participant tracks and source segments a mix response lists.

use super::*;

pub(super) async fn participant_metadata(
    pool: &web::Data<Pool<Postgres>>,
    guild_id: i64,
    anchor: Option<(i64, i64, &[AudioFragment])>,
    contributors: &[MixContributor],
) -> Result<Vec<ChannelMixParticipant>, AppError> {
    let mut by_user: HashMap<i64, ChannelMixParticipant> = HashMap::new();
    if let Some((anchor_user_id, anchor_session_id, anchor_fragments)) = anchor {
        let entry = by_user
            .entry(anchor_user_id)
            .or_insert_with(|| ChannelMixParticipant {
                user_id: anchor_user_id.to_string(),
                display_name: None,
                session_ids: vec![anchor_session_id.to_string()],
                source_count: 0,
            });
        let mut anchor_sources = HashSet::new();
        for fragment in anchor_fragments {
            anchor_sources.insert(fragment.id);
        }
        entry.source_count = anchor_sources.len().try_into().unwrap_or(i32::MAX);
    }
    for contributor in contributors {
        let entry = by_user
            .entry(contributor.user_id)
            .or_insert_with(|| ChannelMixParticipant {
                user_id: contributor.user_id.to_string(),
                display_name: None,
                session_ids: Vec::new(),
                source_count: 0,
            });
        if let Some(session_id) = contributor.session_id
            && !entry.session_ids.contains(&session_id.to_string())
        {
            entry.session_ids.push(session_id.to_string());
        }
        entry.source_count = entry.source_count.saturating_add(1);
    }
    if by_user.is_empty() {
        return Ok(Vec::new());
    }

    let user_ids = by_user.keys().copied().collect::<Vec<_>>();
    let rows = sqlx::query!(
        r#"SELECT un.user_id AS "user_id!",
                COALESCE(nn.nickname, un.global_name, un.username) AS display_name
           FROM user_names un
           LEFT JOIN user_nicknames nn
             ON nn.user_id = un.user_id AND nn.guild_id = $1
          WHERE un.user_id = ANY($2)"#,
        guild_id,
        &user_ids
    )
    .fetch_all(pool.get_ref())
    .await?;
    for row in rows {
        let user_id = row.user_id;
        if let Some(participant) = by_user.get_mut(&user_id) {
            participant.display_name = row.display_name;
        }
    }

    let mut result = by_user.into_values().collect::<Vec<_>>();
    result.sort_by_key(|participant| participant.user_id.parse::<i64>().unwrap_or(i64::MAX));
    for participant in &mut result {
        participant.session_ids.sort();
    }
    Ok(result)
}

pub(super) fn build_tracks(
    participants: &[ChannelMixParticipant],
    sources: &[MixSource],
    anchor_user_id: Option<i64>,
    timeline_start_ms: i64,
) -> Vec<ChannelMixTrack> {
    let mut tracks = participants
        .iter()
        .filter_map(|participant| {
            let user_id = participant.user_id.parse::<i64>().ok()?;
            let mut segments = sources
                .iter()
                .filter(|source| source.participant_user_id == user_id)
                .map(|source| source_segment(source, timeline_start_ms))
                .collect::<Vec<_>>();
            segments.sort_by_key(|segment| {
                (
                    segment.start_ms,
                    segment.end_ms,
                    segment.audio_file_id.clone(),
                )
            });
            Some(ChannelMixTrack {
                user_id: participant.user_id.clone(),
                display_name: participant.display_name.clone(),
                is_anchor: anchor_user_id == Some(user_id),
                segments,
            })
        })
        .collect::<Vec<_>>();
    tracks.sort_by_key(|track| (if track.is_anchor { 0 } else { 1 }, track.user_id.clone()));
    tracks
}

pub(super) fn source_segment(
    source: &MixSource,
    anchor_started_at_ms: i64,
) -> ChannelMixSourceSegment {
    let key = RecordingKey::new(
        source.guild_id,
        source.channel_id,
        source.year,
        source.month as u32,
        source.file_name.clone(),
    );
    let (media_url, hls_playlist_url) = match source.recording_session_id {
        Some(session_id) => (
            format!(
                "/api/audio/sessions/{session_id}/segments/{}",
                source.audio_file_id
            ),
            format!(
                "/api/audio/sessions/{session_id}/live/{}/playlist.m3u8",
                source.audio_file_id
            ),
        ),
        None => (
            key.audio_url(),
            format!(
                "/api/audio/live/{}/{}/{:04}/{:02}/{}/playlist.m3u8",
                key.guild_id, key.channel_id, key.year, key.month, key.stem
            ),
        ),
    };
    let start_ms = source.overlap_start_ms.saturating_sub(anchor_started_at_ms);
    let end_ms = source.overlap_end_ms.saturating_sub(anchor_started_at_ms);
    ChannelMixSourceSegment {
        id: format!("{}:{}", source.audio_file_id, source.overlap_start_ms),
        audio_file_id: source.audio_file_id.to_string(),
        recording_session_id: source.recording_session_id.map(|id| id.to_string()),
        start_ms,
        end_ms,
        source_offset_ms: source
            .overlap_start_ms
            .saturating_sub(source.source_start_ms),
        source_duration_ms: source
            .fragment_end_ms
            .saturating_sub(source.fragment_start_ms),
        live: source.live,
        media_url,
        hls_playlist_url,
        waveform_url: key.waveform_url(),
    }
}

//! Planning a mix: which recordings contribute, over which windows of the
//! anchor session's timeline, and at what offsets.

use super::*;

pub(super) async fn build_mix_plan(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    viewer: crate::permissions::Viewer,
    scope: ChannelMixScope,
) -> Result<MixPlan, AppError> {
    let selected_timeline_end_ms = super::super::timeline_end_ms(access);
    let selected_fragments = measure("fragments", load_fragments(pool, access.session_id))
        .await?
        .into_iter()
        .filter(|fragment| {
            fragment.channel_id != 0
                && fragment.start_ms < selected_timeline_end_ms
                && fragment.end_ms.unwrap_or(selected_timeline_end_ms) > access.started_at_ms
        })
        .collect::<Vec<_>>();
    let (timeline_start_ms, timeline_end_ms, windows, source_anchor_fragments, inputs) = match scope
    {
        ChannelMixScope::SelectedSession => {
            let windows =
                fallback_mix_windows(access, &selected_fragments, selected_timeline_end_ms);
            let inputs = measure(
                "candidates",
                load_mix_candidates(
                    pool,
                    access,
                    &windows,
                    access.started_at_ms,
                    selected_timeline_end_ms,
                    Some(access.user_id),
                ),
            )
            .await?;
            (
                access.started_at_ms,
                selected_timeline_end_ms,
                windows,
                selected_fragments.clone(),
                inputs,
            )
        }
        ChannelMixScope::AllRecordings => {
            let mut windows = measure(
                "windows",
                load_bot_occupancy_windows(
                    pool,
                    access,
                    &selected_fragments,
                    selected_timeline_end_ms,
                ),
            )
            .await?;
            if windows.is_empty() {
                windows =
                    fallback_mix_windows(access, &selected_fragments, selected_timeline_end_ms);
            }
            let timeline_start_ms = windows
                .iter()
                .map(|window| window.start_ms)
                .min()
                .unwrap_or(access.started_at_ms);
            let timeline_end_ms = windows
                .iter()
                .map(|window| window.end_ms)
                .max()
                .unwrap_or(selected_timeline_end_ms);
            let inputs = measure(
                "candidates",
                load_mix_candidates(
                    pool,
                    access,
                    &windows,
                    timeline_start_ms,
                    timeline_end_ms,
                    None,
                ),
            )
            .await?;
            (
                timeline_start_ms,
                timeline_end_ms,
                windows,
                Vec::new(),
                inputs,
            )
        }
    };

    // Authorize every complete logical contributor before resolving display
    // names or returning the participant list. Legacy rows have no logical
    // parent; their channel permission is the only safe authorization scope.
    let mut authorized_sessions = HashSet::new();
    for candidate in &inputs.candidates {
        if let Some(session_id) = candidate.fragment.recording_session_id {
            if session_id == access.session_id || !authorized_sessions.insert(session_id) {
                continue;
            }
            require_session_access(pool, session_id, viewer).await?;
        } else {
            require_channel_access(
                pool,
                candidate.fragment.guild_id,
                candidate.fragment.channel_id,
                viewer,
            )
            .await?;
        }
    }

    let contributors = inputs
        .candidates
        .iter()
        .filter(|candidate| {
            scope == ChannelMixScope::AllRecordings
                || candidate.fragment.recording_session_id != Some(access.session_id)
        })
        .map(|candidate| MixContributor {
            session_id: candidate.fragment.recording_session_id,
            user_id: candidate.fragment.user_id,
            state: candidate.state.clone(),
            fragment: candidate.fragment.clone(),
        })
        .collect::<Vec<_>>();

    let duration_ms = timeline_end_ms.saturating_sub(timeline_start_ms);
    let mut sources = Vec::new();
    if scope == ChannelMixScope::SelectedSession {
        add_window_sources(
            &mut sources,
            &windows,
            &source_anchor_fragments,
            access.user_id,
            timeline_start_ms,
            timeline_end_ms,
        );
        for contributor in &contributors {
            add_window_sources(
                &mut sources,
                &windows,
                std::slice::from_ref(&contributor.fragment),
                contributor.user_id,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    } else {
        for candidate in &inputs.candidates {
            add_window_sources(
                &mut sources,
                &windows,
                std::slice::from_ref(&candidate.fragment),
                candidate.fragment.user_id,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    }

    let participants = measure(
        "participants",
        participant_metadata(
            pool,
            access.guild_id,
            (scope == ChannelMixScope::SelectedSession).then_some((
                access.user_id,
                access.session_id,
                source_anchor_fragments.as_slice(),
            )),
            &contributors,
        ),
    )
    .await?;
    let tracks = build_tracks(
        &participants,
        &sources,
        (scope == ChannelMixScope::SelectedSession).then_some(access.user_id),
        timeline_start_ms,
    );
    let cache_dir = mix_cache_dir(access, &selected_fragments, scope);
    let source_fingerprint = mix_source_fingerprint(
        access,
        scope,
        timeline_start_ms,
        duration_ms,
        &windows,
        &source_anchor_fragments,
        &contributors,
    );
    let settings = default_generation_settings(&tracks);
    let fingerprint = mix_fingerprint(&source_fingerprint, &settings);

    Ok(MixPlan {
        session_id: access.session_id,
        scope,
        duration_ms,
        contributors,
        sources,
        participants,
        tracks,
        cache_dir,
        source_fingerprint,
        fingerprint,
        settings,
    })
}

async fn load_mix_candidates(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    windows: &[MixWindow],
    timeline_start_ms: i64,
    timeline_end_ms: i64,
    excluded_user_id: Option<i64>,
) -> Result<MixPlanInputs, AppError> {
    if windows.is_empty() || timeline_end_ms <= timeline_start_ms {
        return Ok(MixPlanInputs {
            candidates: Vec::new(),
        });
    }

    let rows = sqlx::query!(
        r#"SELECT af.id,
                af.guild_id,
                af.channel_id,
                af.user_id,
                af.recording_session_id,
                af.file_name,
                af.year,
                af.month,
                af.start_ts,
                af.end_ts,
                af.segment_index,
                (
                    af.end_ts IS NULL
                    AND af.reaped IS FALSE
                    AND EXISTS (
                        SELECT 1
                          FROM bot_instances bi
                         WHERE bi.instance_id = af.recording_owner_instance_id
                           AND af.recording_heartbeat_at > now() - interval '120 seconds'
                           AND bi.heartbeat_at > now() - interval '120 seconds'
                           AND bi.state <> 'stopped'
                    )
                ) AS "live!",
                COALESCE(rs.state,
                         CASE WHEN af.end_ts IS NULL THEN 'active' ELSE 'finalized' END) AS "session_state!"
           FROM audio_files af
           LEFT JOIN recording_sessions rs ON rs.id = af.recording_session_id
          WHERE af.guild_id = $1
            AND ($2::bigint IS NULL OR af.user_id <> $2)
            AND af.start_ts IS NOT NULL
            AND af.start_ts < $4
            AND COALESCE(af.end_ts, $4) > $3
          ORDER BY af.start_ts, af.id"#,
        access.guild_id,
        excluded_user_id,
        timeline_start_ms,
        timeline_end_ms
    )
    .fetch_all(pool.get_ref())
    .await?;

    let mut candidates = Vec::new();
    for row in rows {
        let fragment = AudioFragment {
            id: row.id,
            guild_id: row.guild_id,
            channel_id: row.channel_id,
            user_id: row.user_id,
            recording_session_id: row.recording_session_id,
            file_name: row.file_name,
            year: row.year,
            month: row.month,
            start_ms: row.start_ts.unwrap_or(0),
            end_ms: row.end_ts,
            segment_index: row.segment_index,
            live: row.live,
        };
        let overlaps_window = windows.iter().any(|window| {
            if window.channel_id != fragment.channel_id {
                return false;
            }
            let Some(fragment_interval) = MixInterval::new(
                fragment.start_ms,
                fragment.end_ms.unwrap_or(timeline_end_ms),
            ) else {
                return false;
            };
            intersect_mix_intervals(window.interval(), fragment_interval).is_some()
        });
        if overlaps_window {
            candidates.push(CandidateRow {
                fragment,
                state: row.session_state,
            });
        }
    }
    Ok(MixPlanInputs { candidates })
}

#[cfg(test)]
pub(super) fn add_fragment_sources(
    sources: &mut Vec<MixSource>,
    anchor_fragments: &[AudioFragment],
    source_fragments: &[AudioFragment],
    participant_user_id: i64,
    anchor_started_at_ms: i64,
    timeline_end_ms: i64,
) {
    for anchor in anchor_fragments {
        let Some(anchor_interval) = MixInterval::new(
            anchor.start_ms.max(anchor_started_at_ms),
            anchor
                .end_ms
                .unwrap_or(timeline_end_ms)
                .min(timeline_end_ms),
        ) else {
            continue;
        };
        for source in source_fragments {
            if source.channel_id != anchor.channel_id {
                continue;
            }
            let Some(source_interval) = MixInterval::new(
                source.start_ms,
                source.end_ms.unwrap_or(anchor_interval.end_ms),
            ) else {
                continue;
            };
            let Some(overlap) = intersect_mix_intervals(anchor_interval.clone(), source_interval)
            else {
                continue;
            };
            add_source_overlap(
                sources,
                source,
                participant_user_id,
                overlap,
                anchor_started_at_ms,
                timeline_end_ms,
            );
        }
    }
}

fn add_window_sources(
    sources: &mut Vec<MixSource>,
    windows: &[MixWindow],
    source_fragments: &[AudioFragment],
    participant_user_id: i64,
    timeline_start_ms: i64,
    timeline_end_ms: i64,
) {
    for source in source_fragments {
        for window in windows {
            if source.channel_id != window.channel_id {
                continue;
            }
            let Some(source_interval) =
                MixInterval::new(source.start_ms, source.end_ms.unwrap_or(timeline_end_ms))
            else {
                continue;
            };
            let Some(overlap) = intersect_mix_intervals(window.interval(), source_interval) else {
                continue;
            };
            add_source_overlap(
                sources,
                source,
                participant_user_id,
                overlap,
                timeline_start_ms,
                timeline_end_ms,
            );
        }
    }
}

fn add_source_overlap(
    sources: &mut Vec<MixSource>,
    source: &AudioFragment,
    participant_user_id: i64,
    overlap: MixInterval,
    timeline_start_ms: i64,
    timeline_end_ms: i64,
) {
    sources.push(MixSource {
        audio_file_id: source.id,
        recording_session_id: source.recording_session_id,
        participant_user_id,
        guild_id: source.guild_id,
        channel_id: source.channel_id,
        year: source.year,
        month: source.month,
        file_name: source.file_name.clone(),
        path: fragment_path(source),
        fragment_start_ms: source.start_ms,
        fragment_end_ms: source.end_ms.unwrap_or(timeline_end_ms),
        live: source.live,
        source_start_ms: source.start_ms,
        overlap_start_ms: overlap.start_ms,
        overlap_end_ms: overlap.end_ms,
        delay_ms: overlap.start_ms.saturating_sub(timeline_start_ms),
    });
}

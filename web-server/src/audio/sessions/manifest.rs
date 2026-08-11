use super::*;

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/manifest",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Logical recording manifest", body = SessionManifestDto),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/manifest")]
pub async fn get_session_manifest(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    Ok(HttpResponse::Ok().json(build_manifest(&pool, access).await?))
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/events",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Unified logical timeline events", body = [SessionTimelineEventDto]),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/events")]
pub async fn get_session_events(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, token.user_id).await?;
    let timeline_end_ms = timeline_end_ms(&access);
    Ok(HttpResponse::Ok().json(load_events(&pool, &access, timeline_end_ms).await?))
}

#[route(
    "/audio/sessions/{recording_session_id}/segments/{audio_file_id}",
    method = "GET",
    method = "HEAD"
)]
pub async fn get_session_segment(
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
) -> Result<impl Responder, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id) = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let path = fragment_path(&fragment);
    if let Ok(file) = NamedFile::open_async(path).await {
        return Ok(file.into_response(&req));
    }
    media
        .serve_recording(
            &req,
            pool.get_ref(),
            audio_file_id,
            RemoteDisposition::Inline,
        )
        .await
}

#[get("/audio/sessions/{recording_session_id}/live/{audio_file_id}/playlist.m3u8")]
pub async fn session_live_playlist(
    path: web::Path<(i64, i64)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    container: web::Data<LiveContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id) = path.into_inner();
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let key = fragment_key(&fragment);
    let _ = super::super::live::ensure_job(container, pool, key.clone()).await?;
    super::super::live::mark_cache_access(&key.live_dir(&recording_path())).await;
    let playlist = key.live_playlist_path(&recording_path());
    let body = tokio::fs::read(playlist)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let finalized = std::str::from_utf8(&body)
        .map(|contents| contents.contains("#EXT-X-ENDLIST"))
        .unwrap_or(false);
    Ok(HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header((
            header::CACHE_CONTROL,
            if finalized {
                "public, max-age=300"
            } else {
                "no-cache"
            },
        ))
        .body(body))
}

#[get("/audio/sessions/{recording_session_id}/live/{audio_file_id}/{segment}")]
pub async fn session_live_segment(
    req: HttpRequest,
    path: web::Path<(i64, i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<impl Responder, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let (session_id, audio_file_id, segment) = path.into_inner();
    validate_segment_name(&segment)?;
    require_session_access(&pool, session_id, token.user_id).await?;
    let fragment = load_fragment(&pool, session_id, audio_file_id).await?;
    let key = fragment_key(&fragment);
    super::super::live::mark_cache_access(&key.live_dir(&recording_path())).await;
    let path = key.live_segment_path(&recording_path(), &segment);
    let file = NamedFile::open_async(path)
        .await
        .map_err(|_| AppError::FileNotFound)?;
    let mut response = file.into_response(&req);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(if segment.starts_with("seg_") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        }),
    );
    Ok(response)
}

pub(super) async fn build_manifest(
    pool: &web::Data<Pool<Postgres>>,
    access: SessionAccess,
) -> Result<SessionManifestDto, AppError> {
    let fragments = load_fragments(pool, access.session_id).await?;
    let gaps = load_gaps(pool, access.session_id).await?;
    let end_ms = timeline_end_ms(&access).max(
        fragments
            .iter()
            .filter_map(|fragment| fragment.end_ms)
            .max()
            .unwrap_or(access.started_at_ms),
    );
    let mut segments = Vec::with_capacity(fragments.len() + gaps.len());
    for fragment in &fragments {
        let fragment_end = fragment.end_ms.unwrap_or(end_ms.max(fragment.start_ms));
        segments.push(SessionSegmentDto {
            kind: if fragment.live {
                "active_hls".to_string()
            } else {
                "audio".to_string()
            },
            start_ms: fragment.start_ms.saturating_sub(access.started_at_ms),
            end_ms: fragment_end.saturating_sub(access.started_at_ms),
            channel_id: Some(fragment.channel_id.to_string()),
            from_channel_id: None,
            to_channel_id: None,
            audio_file_id: Some(fragment.id.to_string()),
            file_name: Some(fragment.file_name.clone()),
            segment_index: fragment.segment_index,
            reason: None,
            media_url: Some(format!(
                "/api/audio/sessions/{}/segments/{}",
                access.session_id, fragment.id
            )),
            hls_playlist_url: fragment.live.then(|| {
                format!(
                    "/api/audio/sessions/{}/live/{}/playlist.m3u8",
                    access.session_id, fragment.id
                )
            }),
        });
    }
    for gap in gaps {
        segments.push(SessionSegmentDto {
            kind: "silence".to_string(),
            start_ms: gap.start_ms.saturating_sub(access.started_at_ms),
            end_ms: gap.end_ms.saturating_sub(access.started_at_ms),
            channel_id: None,
            from_channel_id: gap.from_channel_id.map(|id| id.to_string()),
            to_channel_id: gap.to_channel_id.map(|id| id.to_string()),
            audio_file_id: None,
            file_name: None,
            segment_index: None,
            reason: Some(gap.reason),
            media_url: None,
            hls_playlist_url: None,
        });
    }
    segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));

    let mut channel_journey = vec![access.starting_channel_id.to_string()];
    for fragment in &fragments {
        let channel_id = fragment.channel_id.to_string();
        if channel_journey.last() != Some(&channel_id) {
            channel_journey.push(channel_id);
        }
    }
    let current_channel_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT current_channel_id FROM recording_sessions WHERE id = $1",
    )
    .bind(access.session_id)
    .fetch_optional(pool.get_ref())
    .await?
    .flatten();
    if let Some(current_channel_id) = current_channel_id {
        let current_channel_id = current_channel_id.to_string();
        if channel_journey.last() != Some(&current_channel_id) {
            channel_journey.push(current_channel_id);
        }
    }

    Ok(SessionManifestDto {
        recording_session_id: access.session_id.to_string(),
        guild_id: access.guild_id.to_string(),
        user_id: access.user_id.to_string(),
        state: access.state.clone(),
        started_at_ms: access.started_at_ms,
        ended_at_ms: access.ended_at_ms,
        duration_ms: end_ms.saturating_sub(access.started_at_ms),
        starting_channel_id: access.starting_channel_id.to_string(),
        current_channel_id: current_channel_id.map(|id| id.to_string()),
        channel_journey,
        events: load_events(pool, &access, end_ms).await?,
        segments,
    })
}

pub(super) async fn load_fragments(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
) -> Result<Vec<AudioFragment>, AppError> {
    let rows = sqlx::query(
        "SELECT af.id,
                af.guild_id,
                af.channel_id,
                af.file_name,
                af.year,
                af.month,
                COALESCE(af.start_ts, 0) AS start_ms,
                af.end_ts AS end_ms,
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
                ) AS live
           FROM audio_files af
          WHERE af.recording_session_id = $1
          ORDER BY af.segment_index NULLS LAST, af.start_ts, af.id",
    )
    .bind(session_id)
    .fetch_all(pool.get_ref())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(AudioFragment {
                id: row.try_get("id")?,
                guild_id: row.try_get("guild_id")?,
                channel_id: row.try_get("channel_id")?,
                file_name: row.try_get("file_name")?,
                year: row.try_get("year")?,
                month: row.try_get("month")?,
                start_ms: row.try_get("start_ms")?,
                end_ms: row.try_get("end_ms")?,
                segment_index: row.try_get("segment_index")?,
                live: row.try_get::<Option<bool>, _>("live")?.unwrap_or(false),
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(AppError::DbError)
}

pub(super) async fn load_fragment(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
    audio_file_id: i64,
) -> Result<AudioFragment, AppError> {
    load_fragments(pool, session_id)
        .await?
        .into_iter()
        .find(|fragment| fragment.id == audio_file_id)
        .ok_or(AppError::FileNotFound)
}

pub(super) async fn load_gaps(
    pool: &web::Data<Pool<Postgres>>,
    session_id: i64,
) -> Result<Vec<Gap>, AppError> {
    let rows = sqlx::query(
        "SELECT (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS start_ms,
                (EXTRACT(EPOCH FROM ended_at) * 1000)::bigint AS end_ms,
                reason,
                from_channel_id,
                to_channel_id
           FROM recording_gaps
          WHERE recording_session_id = $1
          ORDER BY started_at, id",
    )
    .bind(session_id)
    .fetch_all(pool.get_ref())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(Gap {
                start_ms: row.try_get("start_ms")?,
                end_ms: row.try_get("end_ms")?,
                reason: row.try_get("reason")?,
                from_channel_id: row.try_get("from_channel_id")?,
                to_channel_id: row.try_get("to_channel_id")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(AppError::DbError)
}

pub(super) async fn load_events(
    pool: &web::Data<Pool<Postgres>>,
    access: &SessionAccess,
    timeline_end_ms: i64,
) -> Result<Vec<SessionTimelineEventDto>, AppError> {
    let mut events = Vec::new();
    let session_rows = sqlx::query(
        "SELECT event_type,
                ((EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint - $2) AS offset_ms,
                channel_id,
                previous_channel_id,
                details
           FROM recording_session_events
          WHERE recording_session_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(access.session_id)
    .bind(access.started_at_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in session_rows {
        events.push(SessionTimelineEventDto {
            source: "recording".to_string(),
            event_type: row.try_get("event_type")?,
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("previous_channel_id")?
                .map(|id| id.to_string()),
            details: row.try_get("details")?,
        });
    }

    let voice_rows = sqlx::query(
        "SELECT t.name AS event_type,
                ((EXTRACT(EPOCH FROM v.occurred_at) * 1000)::bigint - $2) AS offset_ms,
                v.channel_id,
                v.previous_channel_id
           FROM voice_state_events v
           JOIN voice_state_event_types t ON t.id = v.event_type_id
          WHERE v.guild_id = $1
            AND v.user_id = $3
            AND v.occurred_at >= to_timestamp($2::double precision / 1000.0)
            AND v.occurred_at <= to_timestamp($4::double precision / 1000.0)
          ORDER BY v.occurred_at, v.id",
    )
    .bind(access.guild_id)
    .bind(access.started_at_ms)
    .bind(access.user_id)
    .bind(timeline_end_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in voice_rows {
        events.push(SessionTimelineEventDto {
            source: "voice_state".to_string(),
            event_type: row.try_get("event_type")?,
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("previous_channel_id")?
                .map(|id| id.to_string()),
            details: serde_json::json!({}),
        });
    }

    let connection_rows = sqlx::query(
        "SELECT outcome,
                trigger,
                owner_instance_id,
                release_id,
                ((EXTRACT(EPOCH FROM started_at) * 1000)::bigint - $2) AS started_offset_ms,
                ((EXTRACT(EPOCH FROM completed_at) * 1000)::bigint - $2) AS offset_ms,
                from_channel_id,
                to_channel_id,
                operation_id,
                error,
                fallback_outcome,
                fallback_error,
                population_snapshot
           FROM voice_connection_events
          WHERE guild_id = $1
            AND completed_at >= to_timestamp($2::double precision / 1000.0)
            AND started_at <= to_timestamp($3::double precision / 1000.0)
          ORDER BY completed_at, id",
    )
    .bind(access.guild_id)
    .bind(access.started_at_ms)
    .bind(timeline_end_ms)
    .fetch_all(pool.get_ref())
    .await?;
    for row in connection_rows {
        let trigger: String = row.try_get("trigger")?;
        let outcome: String = row.try_get("outcome")?;
        events.push(SessionTimelineEventDto {
            source: "voice_connection".to_string(),
            event_type: format!("{trigger}:{outcome}"),
            offset_ms: row.try_get("offset_ms")?,
            channel_id: row
                .try_get::<Option<i64>, _>("to_channel_id")?
                .map(|id| id.to_string()),
            previous_channel_id: row
                .try_get::<Option<i64>, _>("from_channel_id")?
                .map(|id| id.to_string()),
            details: serde_json::json!({
                "operation_id": row.try_get::<String, _>("operation_id")?,
                "owner_instance_id": row.try_get::<Option<String>, _>("owner_instance_id")?,
                "release_id": row.try_get::<Option<String>, _>("release_id")?,
                "started_offset_ms": row.try_get::<i64, _>("started_offset_ms")?,
                "error": row.try_get::<Option<String>, _>("error")?,
                "fallback_outcome": row.try_get::<Option<String>, _>("fallback_outcome")?,
                "fallback_error": row.try_get::<Option<String>, _>("fallback_error")?,
                "population_snapshot": row.try_get::<serde_json::Value, _>("population_snapshot")?,
            }),
        });
    }
    events.sort_by_key(|event| event.offset_ms);
    Ok(events)
}

pub(super) fn timeline_end_ms(access: &SessionAccess) -> i64 {
    match access.state.as_str() {
        "finalized" => access.ended_at_ms.unwrap_or(access.started_at_ms),
        "pending" => access.pause_started_at_ms.unwrap_or(access.started_at_ms),
        _ => chrono::Utc::now().timestamp_millis(),
    }
}

pub(super) fn fragment_key(fragment: &AudioFragment) -> RecordingKey {
    RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    )
}

pub(super) fn fragment_path(fragment: &AudioFragment) -> PathBuf {
    RecordingKey::new(
        fragment.guild_id,
        fragment.channel_id,
        fragment.year,
        fragment.month as u32,
        fragment.file_name.clone(),
    )
    .recording_path(&recording_path())
}

pub(super) fn validate_segment_name(segment: &str) -> Result<(), AppError> {
    if segment.is_empty()
        || segment == "playlist.m3u8"
        || segment == "state"
        || segment.contains('/')
        || segment.contains("..")
        || segment.contains('\\')
    {
        Err(AppError::BadRequest("Invalid HLS segment name".into()))
    } else {
        Ok(())
    }
}

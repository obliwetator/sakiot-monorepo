use super::*;

#[utoipa::path(
    post,
    path = "/api/audio/clips/{guild_id}/compose",
    tag = "clips",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = ComposeClipBody,
    responses(
        (status = 202, description = "Clip composition started", body = ComposeClipAccepted),
        (status = 400, description = "Invalid composition request", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 404, description = "Source clip not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/clips/{guild_id}/compose")]
pub async fn compose_clip(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ComposeClipBody>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    media: web::Data<MediaArchive>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let user_id = token.user_id;
    let guild_id = path.into_inner();

    let validated = validate_composition(body.into_inner())?;
    let ResolvedComposition {
        validated: ValidatedComposition(body),
        sources: resolved,
    } = resolve_composition(&pool, &media, guild_id, user_id, validated).await?;
    let overwrite = match body.overwrite_clip_id.as_deref() {
        Some(target_id) => {
            let row = sqlx::query(
                "SELECT original_file_name, user_id, saved_file_name, name
                   FROM clips
                  WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
            )
            .bind(guild_id)
            .bind(target_id)
            .fetch_optional(pool.get_ref())
            .await?
            .ok_or(AppError::ClipNotFound)?;
            if row
                .try_get::<Option<String>, _>("original_file_name")?
                .as_deref()
                != Some("compose")
            {
                return Err(AppError::BadRequest(
                    "Only composed clips can be overwritten".into(),
                ));
            }
            if row.try_get::<Option<i64>, _>("user_id")? != Some(user_id) {
                require_guild_manager(&req, &pool, guild_id).await?;
            }
            Some(ComposeOverwrite {
                clip_id: target_id.to_string(),
                old_saved_file_name: row
                    .try_get::<Option<String>, _>("saved_file_name")?
                    .ok_or(AppError::ClipNotFound)?,
                fallback_name: row.try_get::<Option<String>, _>("name")?,
            })
        }
        None => None,
    };
    let channel_id = resolved
        .first()
        .map(|source| source.channel_id)
        .unwrap_or_default();

    let segments: Vec<SegmentRender> = body
        .segments
        .iter()
        .zip(resolved)
        .map(|(segment, source)| SegmentRender {
            path: source.path,
            source_in: segment.source_in,
            source_out: segment.source_out,
            effects: shared_effects_from_dto(&segment.effects),
            timeline_start: segment.timeline_start,
        })
        .collect();
    let expected_total_ms = expected_duration_ms(&segments);
    let name = body
        .name
        .as_deref()
        .and_then(crate::clips::normalized_clip_name)
        .map(str::to_owned)
        .or_else(|| {
            overwrite
                .as_ref()
                .and_then(|target| target.fallback_name.clone())
        })
        .unwrap_or_else(|| "composed-clip".to_string());
    let master_volume_db = body.master_volume_db;
    // The overwrite target and the per-user limits are request metadata, not
    // part of the edit; keep them out of the stored composition so re-imports
    // see the edit alone.
    let mut composition_value = serde_json::to_value(&body).map_err(|_| AppError::InternalError)?;
    if let Some(object) = composition_value.as_object_mut() {
        object.remove("overwrite_clip_id");
        object.remove("limits");
    }
    let composition = composition_value;

    let clip_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let target_dir = PathBuf::from(clips_path())
        .join(now.year().to_string())
        .join(format!("{:02}", now.month()));
    tokio::fs::create_dir_all(&target_dir).await?;
    let full_path = target_dir.join(format!("{clip_id}.ogg"));
    let saved_file_name = format!("{}/{:02}/{clip_id}.ogg", now.year(), now.month());
    let cache_key = compose_progress_key(&clip_id);
    let clip_id_task = clip_id.clone();

    {
        let mut values = progress.0.write().await;
        if values.get(&cache_key).copied() == Some(-1) {
            values.remove(&cache_key);
        }
        values.insert(cache_key.clone(), 0);
    }

    let pool_clone = pool.clone();
    let progress_clone = progress.clone();
    let overwrite_job = overwrite.is_some();
    tokio::spawn(async move {
        progress_clone.0.write().await.insert(cache_key.clone(), 1);
        let result = run_compose_job(
            &pool_clone,
            &segments,
            master_volume_db,
            &full_path,
            expected_total_ms,
            &progress_clone,
            &cache_key,
            &clip_id_task,
            guild_id,
            user_id,
            channel_id,
            &name,
            &saved_file_name,
            &composition,
            overwrite,
        )
        .await;
        match result {
            Ok(()) => {
                if overwrite_job {
                    // The render uuid never lands in the clips table (the row
                    // keeps its original id), so leave a done marker the
                    // status endpoint can report as ready.
                    progress_clone
                        .0
                        .write()
                        .await
                        .insert(cache_key.clone(), 100);
                } else {
                    progress_clone.0.write().await.remove(&cache_key);
                }
            }
            Err(error) => {
                error!(clip_id = %clip_id_task, "clip composition failed: {}", error);
                let _ = tokio::fs::remove_file(&full_path).await;
                progress_clone.0.write().await.insert(cache_key, -1);
            }
        }
    });

    Ok(HttpResponse::Accepted().json(ComposeClipAccepted {
        status: "processing",
        progress: 0,
        id: clip_id,
    }))
}

#[utoipa::path(
    get,
    path = "/api/audio/clips/{guild_id}/compose/{clip_id}",
    tag = "clips",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("clip_id" = String, Path, description = "Composition clip id"),
    ),
    responses(
        (status = 200, description = "Composition status", body = ComposeClipStatus),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/clips/{guild_id}/compose/{clip_id}")]
pub async fn compose_clip_status(
    path: web::Path<(i64, String)>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
    progress: web::Data<WaveformProgressContainer>,
) -> Result<HttpResponse, AppError> {
    let (guild_id, clip_id) = path.into_inner();
    token.ok_or(AppError::Unauthorized)?;
    let cache_key = compose_progress_key(&clip_id);

    {
        let mut map = progress.0.write().await;
        if let Some(value) = map.get(&cache_key).copied() {
            if value < 0 {
                map.remove(&cache_key);
                return Ok(HttpResponse::Ok().json(ComposeClipStatus {
                    status: "failed".into(),
                    progress: 0,
                }));
            }
            if value >= 100 {
                map.remove(&cache_key);
                return Ok(HttpResponse::Ok().json(ComposeClipStatus {
                    status: "ready".into(),
                    progress: 100,
                }));
            }
            return Ok(HttpResponse::Ok().json(ComposeClipStatus {
                status: "processing".into(),
                progress: value.min(99),
            }));
        }
    }

    let row = sqlx::query(
        "SELECT 1 FROM clips WHERE guild_id = $1 AND clip_id = $2 AND deleted_at IS NULL",
    )
    .bind(guild_id)
    .bind(&clip_id)
    .fetch_optional(pool.get_ref())
    .await?;
    if row.is_some() {
        return Ok(HttpResponse::Ok().json(ComposeClipStatus {
            status: "ready".into(),
            progress: 100,
        }));
    }
    Ok(HttpResponse::Ok().json(ComposeClipStatus {
        status: "idle".into(),
        progress: 0,
    }))
}

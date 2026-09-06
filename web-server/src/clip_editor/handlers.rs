use super::*;

#[utoipa::path(
    post,
    path = "/api/audio/clips/{guild_id}/compose",
    tag = "clips",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("Idempotency-Key" = Option<String>, Header, description = "Stable request key; reuse only with the same export body. Retained for 30 days after completion."),
    ),
    request_body = ComposeClipBody,
    responses(
        (status = 202, description = "Clip composition durably queued", body = ComposeClipAccepted),
        (status = 409, description = "Request key already used for a different export", body = crate::errors::ApiError),
        (status = 503, description = "Export queue unavailable or full", body = crate::errors::ApiError),
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
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let user_id = token.user_id;
    let guild_id = path.into_inner();

    let request = serde_json::to_value(&body.0).map_err(|_| AppError::InternalError)?;
    let key = req
        .headers()
        .get("Idempotency-Key")
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| AppError::BadRequest("Invalid export request key".into()))
        })
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(AppError::BadRequest("Invalid export request key".into()));
    }
    if let Some(id) = queue::existing(&pool, guild_id, user_id, &key, &request).await? {
        return Ok(HttpResponse::Accepted().json(ComposeClipAccepted {
            status: "processing",
            progress: 0,
            id,
        }));
    }
    let validated = validate_composition(body.into_inner())?;
    let ResolvedComposition {
        validated: ValidatedComposition(body),
        sources: resolved,
    } = resolve_composition(&pool, None, guild_id, user_id, validated).await?;
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
    let snapshot = queue::Snapshot {
        sources: resolved
            .into_iter()
            .map(|source| source.saved_file_name)
            .collect(),
        body,
        overwrite,
        channel_id,
        name,
    };
    let id = queue::enqueue(&pool, guild_id, user_id, &key, &request, &snapshot).await?;
    Ok(HttpResponse::Accepted().json(ComposeClipAccepted {
        status: "processing",
        progress: 0,
        id,
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
        (status = 200, description = "Stable owner-scoped composition status", body = ComposeClipStatus),
        (status = 404, description = "Job not found, expired, or owned by another user", body = crate::errors::ApiError),
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
) -> Result<HttpResponse, AppError> {
    let (guild_id, job_id) = path.into_inner();
    let token = token.ok_or(AppError::Unauthorized)?;
    let status = queue::status(&pool, guild_id, token.user_id, &job_id).await?;
    Ok(HttpResponse::Ok().json(status))
}

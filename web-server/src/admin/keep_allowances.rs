//! How many recordings each member may keep within the server's cap
//! ([`crate::keeping`]), for holders of `manage_settings`: an allowance for
//! @everyone, a role or a member. A member's own allowance wins, else the
//! largest of their roles', else @everyone's; with none, only the cap
//! limits them.

use actix_web::{HttpRequest, HttpResponse, get, put, web};
use sakiot_permissions::{Capabilities, Subject};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use super::permission_rules::{RuleRole, RuleSubject, SubjectKind};
use crate::errors::AppError;
use crate::permissions::require_capability;

/// The largest allowance, and the largest cap an operator may set.
pub const MAX_KEPT: i32 = 100_000;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeepAllowance {
    pub subject: RuleSubject,
    /// The role's or member's name; null for everyone, a role that is gone
    /// or a user never seen.
    pub name: Option<String>,
    pub allowance: i32,
    /// Its role is gone, so it reaches nobody; only removing it is left.
    pub reaches_nobody: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeepAllowances {
    /// How many recordings the server may keep, as the bot's operators set
    /// it.
    pub cap: i32,
    /// How many it keeps, each once however many members keep it.
    pub kept: i64,
    /// @everyone's first, then roles' and members'.
    pub allowances: Vec<KeepAllowance>,
    /// The server's roles, but `@everyone`.
    pub roles: Vec<RuleRole>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct KeepAllowanceChange {
    pub subject: RuleSubject,
    /// Null removes the subject's allowance.
    pub allowance: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/keep-allowances",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "The server's cap and how many recordings each member may keep", body = KeepAllowances),
        (status = 403, description = "Manage settings required", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/keep-allowances")]
pub async fn get_keep_allowances(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    require_capability(&req, &pool, guild_id, Capabilities::MANAGE_SETTINGS).await?;
    let usage = sqlx::query!(
        r#"SELECT COALESCE((SELECT kept_recordings FROM guild_limits WHERE guild_id = $1), 0)
                      AS "cap!",
                  (SELECT count(DISTINCT recording_session_id)
                     FROM kept_recordings
                    WHERE guild_id = $1) AS "kept!""#,
        guild_id
    )
    .fetch_one(pool.get_ref())
    .await?;
    let allowances = sqlx::query!(
        r#"SELECT a.subject_kind, a.subject_id, a.allowance,
                  CASE a.subject_kind
                      WHEN 'role' THEN (SELECT r.name FROM roles r
                                         WHERE r.role_id = a.subject_id
                                           AND r.guild_id = a.guild_id)
                      WHEN 'user' THEN (SELECT COALESCE(nn.nickname, un.global_name, un.username)
                                          FROM user_names un
                                          LEFT JOIN user_nicknames nn
                                            ON nn.user_id = un.user_id
                                           AND nn.guild_id = a.guild_id
                                         WHERE un.user_id = a.subject_id)
                  END AS name,
                  a.subject_kind = 'role'
                  AND NOT EXISTS (SELECT 1 FROM roles r
                                   WHERE r.role_id = a.subject_id
                                     AND r.guild_id = a.guild_id) AS "reaches_nobody!"
             FROM keep_allowances a
            WHERE a.guild_id = $1
            ORDER BY array_position(ARRAY['everyone', 'role', 'user'], a.subject_kind),
                     a.subject_id"#,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?
    .into_iter()
    .map(|row| KeepAllowance {
        subject: RuleSubject {
            kind: match row.subject_kind.as_str() {
                "role" => SubjectKind::Role,
                "user" => SubjectKind::User,
                _ => SubjectKind::Everyone,
            },
            id: row.subject_id,
        },
        name: row.name,
        allowance: row.allowance,
        reaches_nobody: row.reaches_nobody,
    })
    .collect();
    let roles = sqlx::query_as!(
        RuleRole,
        "SELECT role_id, name, color, color_secondary, color_tertiary
           FROM roles
          WHERE guild_id = $1 AND role_id <> $1
          ORDER BY role_id",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(KeepAllowances {
        cap: usage.cap,
        kept: usage.kept,
        allowances,
        roles,
    }))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/keep-allowances",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = KeepAllowanceChange,
    responses(
        (status = 204, description = "Allowance set, or removed when null"),
        (status = 400, description = "Invalid subject or allowance", body = crate::errors::ApiError),
        (status = 403, description = "Manage settings required", body = crate::errors::ApiError),
        (status = 404, description = "The role is not this server's", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/keep-allowances")]
pub async fn put_keep_allowance(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<KeepAllowanceChange>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let user_id = require_capability(&req, &pool, guild_id, Capabilities::MANAGE_SETTINGS)
        .await?
        .user_id;
    let subject = body.subject.subject(guild_id)?;
    let (kind, id) = subject.stored();
    let Some(allowance) = body.allowance else {
        sqlx::query!(
            "DELETE FROM keep_allowances
              WHERE guild_id = $1 AND subject_kind = $2 AND subject_id IS NOT DISTINCT FROM $3",
            guild_id,
            kind,
            id
        )
        .execute(pool.get_ref())
        .await?;
        return Ok(HttpResponse::NoContent().finish());
    };
    if !(0..=MAX_KEPT).contains(&allowance) {
        return Err(AppError::BadRequest(format!(
            "An allowance must be between 0 and {MAX_KEPT} recordings."
        )));
    }
    if let Subject::Role(role_id) = subject {
        let known = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM roles WHERE guild_id = $1 AND role_id = $2) AS "known!""#,
            guild_id,
            role_id
        )
        .fetch_one(pool.get_ref())
        .await?;
        if !known {
            return Err(AppError::RoleNotFound);
        }
    }
    sqlx::query!(
        "INSERT INTO keep_allowances (guild_id, subject_kind, subject_id, allowance, updated_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT ON CONSTRAINT keep_allowances_one_per_subject
         DO UPDATE SET allowance = EXCLUDED.allowance,
                       updated_by = EXCLUDED.updated_by,
                       updated_at = now()",
        guild_id,
        kind,
        id,
        allowance,
        user_id
    )
    .execute(pool.get_ref())
    .await?;
    Ok(HttpResponse::NoContent().finish())
}

//! Server-side sign-in sessions (`auth_sessions`). Tokens stay signed JWTs,
//! but a refresh succeeds only while its session row exists, so deleting the
//! row signs that browser out once its access token expires. Access tokens are
//! not checked against the table.

use sqlx::{Pool, Postgres};
use uuid::Uuid;

use super::jwt::{AuthKind, JWT_REFRESH_EXPIRY_DAYS, Refresh, Token};
use crate::errors::AppError;

/// Record a new sign-in and return its id for the tokens. Sessions whose
/// refresh token has expired can never refresh again; sign-in is rare enough
/// to sweep them here.
pub async fn start(
    pool: &Pool<Postgres>,
    user_id: i64,
    auth_kind: AuthKind,
) -> Result<String, AppError> {
    let session_id = Uuid::new_v4().to_string();
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM auth_sessions WHERE refreshed_at < now() - make_interval(days => $1)",
        i32::try_from(JWT_REFRESH_EXPIRY_DAYS).map_err(|_| AppError::InternalError)?,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO auth_sessions (id, user_id, auth_kind) VALUES ($1, $2, $3)",
        session_id,
        user_id,
        auth_kind.as_str(),
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(session_id)
}

/// Whether the refresh token's session still exists; marks it refreshed.
pub async fn refresh(pool: &Pool<Postgres>, token: &Token<Refresh>) -> Result<bool, AppError> {
    let result = sqlx::query!(
        "UPDATE auth_sessions SET refreshed_at = now()
         WHERE id = $1 AND user_id = $2 AND auth_kind = $3",
        token.session_id,
        token.user_id,
        token.auth_kind.as_str(),
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// End one session, as logout does for the browser that asks.
pub async fn end(pool: &Pool<Postgres>, token: &Token<Refresh>) -> Result<(), AppError> {
    sqlx::query!(
        "DELETE FROM auth_sessions WHERE id = $1 AND user_id = $2 AND auth_kind = $3",
        token.session_id,
        token.user_id,
        token.auth_kind.as_str(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// End every session of one identity. A dev login and a Discord login with
/// the same id are different identities, so only `auth_kind`'s sessions go.
pub async fn end_all(
    pool: &Pool<Postgres>,
    user_id: i64,
    auth_kind: AuthKind,
) -> Result<u64, AppError> {
    let result = sqlx::query!(
        "DELETE FROM auth_sessions WHERE user_id = $1 AND auth_kind = $2",
        user_id,
        auth_kind.as_str(),
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

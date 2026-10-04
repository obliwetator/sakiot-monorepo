//! Sign in as a synthetic member, the way Discord OAuth would have.

use actix_web::{HttpRequest, HttpResponse, post, web};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{LoadtestState, authorize, ids};
use crate::auth::cookies::{
    access_token_cookie, csrf_cookie, logged_in_cookie, refresh_token_cookie,
};
use crate::auth::{Access, AccessKeys, AuthKind, Refresh, Token};
use crate::config::Config;
use crate::errors::AppError;

#[derive(Deserialize)]
pub struct LoginRequest {
    /// A string: snowflakes do not survive a JavaScript number.
    user_id: String,
}

#[derive(Serialize)]
struct LoginResponse {
    user_id: String,
    csrf: String,
    access_token: String,
    refresh_token: String,
}

/// Issue a Discord-kind session for a member of a fixture guild. The tokens
/// come back as the usual cookies and, for load generators that manage their
/// own cookies, in the body. Only synthetic members are accepted, so this
/// cannot impersonate a real account.
#[post("/login")]
pub async fn login(
    req: HttpRequest,
    body: web::Json<LoginRequest>,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
    state: web::Data<LoadtestState>,
) -> Result<HttpResponse, AppError> {
    authorize(&req, &cfg)?;
    let user_id: i64 = body
        .user_id
        .parse()
        .map_err(|_| AppError::InvalidParam("user_id".into()))?;
    let guild = ids::guild_of(user_id).ok_or(AppError::Forbidden)?;
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM guild_members WHERE guild_id = $1 AND user_id = $2)",
    )
    .bind(ids::guild_id(guild))
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    if !member {
        return Err(AppError::Forbidden);
    }

    let csrf = Uuid::new_v4().to_string();
    let access_token = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
        csrf.clone(),
        &keys.access_encode,
    )?;
    let refresh_token = Token::<Refresh>::encode(
        user_id,
        AuthKind::Discord,
        csrf.clone(),
        &keys.refresh_encode,
    )?;

    let mut response = HttpResponse::Ok().json(LoginResponse {
        user_id: user_id.to_string(),
        csrf: csrf.clone(),
        access_token: access_token.clone(),
        refresh_token: refresh_token.clone(),
    });
    response.add_cookie(&access_token_cookie(&access_token))?;
    response.add_cookie(&refresh_token_cookie(&refresh_token))?;
    response.add_cookie(&csrf_cookie(&csrf))?;
    response.add_cookie(&logged_in_cookie())?;
    Ok(response)
}

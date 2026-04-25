use actix_files::NamedFile;
use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use reqwest::Client;
use serde_json::json;
use sqlx::{Pool, Postgres};
use tracing::warn;
use uuid::Uuid;

use crate::config::Config;
use crate::errors::AppError;
use crate::user::{get_user, get_user_guilds};

use super::cookies::{
    access_token_cookie, clear_access_token_cookie, clear_csrf_cookie, clear_legacy_access_cookie,
    clear_legacy_refresh_cookie, clear_refresh_token_cookie, csrf_cookie, refresh_token_cookie,
};
use super::discord::{request_access_token, request_refresh_token, DiscordLoginCode};
use super::jwt::{Access, AccessKeys, Refresh, Token};

async fn create_jwt_tokens(
    access_token: String,
    refresh_token: String,
    id: i64,
    csrf: String,
    keys: &web::Data<AccessKeys>,
) -> Result<(String, String), AppError> {
    let access_token =
        Token::<Access>::encode(id, access_token, csrf.clone(), &keys.access_encode)?;
    let refresh_token = Token::<Refresh>::encode(id, refresh_token, csrf, &keys.refresh_encode)?;
    Ok((access_token, refresh_token))
}

#[get("/discord_login")]
pub async fn discord_login(
    req: HttpRequest,
    query: web::Query<DiscordLoginCode>,
    pool: web::Data<Pool<Postgres>>,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    let data = request_access_token(&cfg, query.code.to_owned(), client.clone()).await?;

    let user = get_user(client.clone(), &data.access_token, &pool).await?;
    let _guilds = get_user_guilds(client, &data.access_token, user.id, &pool).await?;

    let csrf_token = Uuid::new_v4().to_string();

    let (access_token, refresh_token) = create_jwt_tokens(
        data.access_token,
        data.refresh_token,
        user.id,
        csrf_token.clone(),
        &keys,
    )
    .await?;
    let mut b = NamedFile::open_async("callback.html")
        .await?
        .into_response(&req);

    b.headers_mut().insert(
        actix_web::http::header::CACHE_CONTROL,
        actix_web::http::header::HeaderValue::from_static(
            "no-store, no-cache, must-revalidate, max-age=0",
        ),
    );

    let d = cfg.cookie_domain.as_str();
    b.add_cookie(&clear_legacy_access_cookie(d))?;
    b.add_cookie(&clear_legacy_refresh_cookie(d))?;
    b.add_cookie(&access_token_cookie(d, &access_token))?;
    b.add_cookie(&refresh_token_cookie(d, &refresh_token))?;
    b.add_cookie(&csrf_cookie(d, &csrf_token))?;

    Ok(b)
}

#[get("/dev_login")]
pub async fn dev_login(
    req: HttpRequest,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    let dev_account_id = cfg.dev_account_id;
    if dev_account_id != 146638124288704513 {
        return Err(AppError::Forbidden);
    }

    let csrf_token = Uuid::new_v4().to_string();

    let (access_token, refresh_token) = create_jwt_tokens(
        "dev_access".into(),
        "dev_refresh".into(),
        dev_account_id,
        csrf_token.clone(),
        &keys,
    )
    .await?;
    let mut b = NamedFile::open_async("callback.html")
        .await?
        .into_response(&req);

    let d = cfg.cookie_domain.as_str();
    b.add_cookie(&clear_legacy_access_cookie(d))?;
    b.add_cookie(&clear_legacy_refresh_cookie(d))?;
    b.add_cookie(&access_token_cookie(d, &access_token))?;
    b.add_cookie(&refresh_token_cookie(d, &refresh_token))?;
    b.add_cookie(&csrf_cookie(d, &csrf_token))?;

    Ok(b)
}

#[get("/refresh")]
pub async fn refresh_jwt(
    req: HttpRequest,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    let refresh_cookie = match req.cookie("refresh_token") {
        Some(c) => c,
        None => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: missing refresh_token cookie",
                req.path()
            );
            return Ok(HttpResponse::Unauthorized().finish());
        }
    };

    let decoded_refresh = match Token::<Refresh>::decode(refresh_cookie.value(), &keys) {
        Ok(token) => token,
        Err(_) => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: expired or invalid refresh token",
                req.path()
            );
            return Ok(HttpResponse::Unauthorized().json(json!({
                "error": "expired_or_invalid_token",
                "message": "The refresh token is expired or invalid. Please login again."
            })));
        }
    };

    let csrf_token = Uuid::new_v4().to_string();

    let (new_access_token, new_refresh_token) = if decoded_refresh.token == "dev_refresh" {
        create_jwt_tokens(
            "dev_access".into(),
            "dev_refresh".into(),
            decoded_refresh.id,
            csrf_token.clone(),
            &keys,
        )
        .await?
    } else {
        let data = request_refresh_token(&cfg, decoded_refresh.token, client.clone()).await?;

        create_jwt_tokens(
            data.access_token,
            data.refresh_token,
            decoded_refresh.id,
            csrf_token.clone(),
            &keys,
        )
        .await?
    };

    let d = cfg.cookie_domain.as_str();
    let mut response = HttpResponse::Ok().json(json!({ "token": new_access_token }));
    response.add_cookie(&access_token_cookie(d, &new_access_token))?;
    response.add_cookie(&refresh_token_cookie(d, &new_refresh_token))?;
    response.add_cookie(&csrf_cookie(d, &csrf_token))?;

    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Cookies cleared"),
    ),
)]
#[get("/logout")]
pub async fn logout(cfg: web::Data<Config>) -> Result<impl Responder, AppError> {
    let d = cfg.cookie_domain.as_str();
    let mut resp = HttpResponse::Ok().finish();
    resp.add_cookie(&clear_access_token_cookie(d))?;
    resp.add_cookie(&clear_refresh_token_cookie(d))?;
    resp.add_cookie(&clear_csrf_cookie(d))?;
    Ok(resp)
}

#[utoipa::path(
    get,
    path = "/api/token",
    tag = "auth",
    responses(
        (status = 200, description = "Returns the current access_token cookie value as JSON", body = serde_json::Value),
        (status = 400, description = "access_token cookie missing"),
    ),
    security(("access_token" = [])),
)]
#[get("/token")]
pub async fn get_token(req: HttpRequest) -> Result<impl Responder, AppError> {
    let access_cookie = req
        .cookie("access_token")
        .ok_or_else(|| AppError::BadRequest("Missing access_token cookie".into()))?;

    let json = json!({ "token": access_cookie.value() });

    Ok(HttpResponse::Ok().json(json))
}

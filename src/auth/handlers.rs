#[cfg(feature = "dev-login")]
use actix_files::NamedFile;
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use reqwest::Client;
use sqlx::{Pool, Postgres};
use tracing::warn;
use uuid::Uuid;

use crate::config::Config;
use crate::errors::AppError;
use crate::user::{get_user, get_user_guilds};

use super::cookies::{
    access_token_cookie, clear_access_token_cookie, clear_csrf_cookie, clear_legacy_access_cookie,
    clear_legacy_refresh_cookie, clear_logged_in_cookie, clear_oauth_state_cookie,
    clear_opener_origin_cookie, clear_refresh_token_cookie, csrf_cookie, logged_in_cookie,
    oauth_state_cookie, opener_origin_cookie, refresh_token_cookie,
};
use super::discord::{request_access_token, DiscordLoginCode};
use super::jwt::{Access, AccessKeys, AuthKind, Refresh, Token};
use subtle::ConstantTimeEq;

async fn create_jwt_tokens(
    id: i64,
    auth_kind: AuthKind,
    csrf: String,
    keys: &web::Data<AccessKeys>,
) -> Result<(String, String), AppError> {
    let access_token = Token::<Access>::encode(id, auth_kind, csrf.clone(), &keys.access_encode)?;
    let refresh_token = Token::<Refresh>::encode(id, auth_kind, csrf, &keys.refresh_encode)?;
    Ok((access_token, refresh_token))
}

#[derive(serde::Deserialize)]
pub struct OauthStartQuery {
    pub origin: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RefreshTokenResponse {
    pub status: &'static str,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RefreshTokenError {
    pub error: &'static str,
    pub message: &'static str,
}

fn origin_matches_host_suffix(origin: &str, suffix: &str) -> bool {
    let suffix = suffix.trim().trim_start_matches('.');
    if suffix.is_empty() {
        return false;
    }

    let Some(host) = origin.strip_prefix("https://") else {
        return false;
    };
    if host.contains('/') || host.contains('?') || host.contains('#') || host.is_empty() {
        return false;
    }

    host == suffix
        || host
            .strip_suffix(&format!(".{suffix}"))
            .is_some_and(|subdomain| !subdomain.is_empty())
}

pub fn is_allowed_opener_origin(origin: &str, cfg: &Config) -> bool {
    if cfg
        .oauth_allowed_opener_origins
        .iter()
        .any(|allowed| origin == allowed)
    {
        return true;
    }

    cfg.oauth_allowed_opener_host_suffixes
        .iter()
        .any(|suffix| origin_matches_host_suffix(origin, suffix))
}

fn csrf_header(req: &HttpRequest) -> Option<&str> {
    req.headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
}

fn require_csrf(req: &HttpRequest, expected: &str) -> Result<(), AppError> {
    let Some(actual) = csrf_header(req) else {
        warn!("CSRF token missing for {}", req.path());
        return Err(AppError::Forbidden);
    };
    let Some(cookie) = req.cookie("xsrf_token") else {
        warn!("CSRF cookie missing for {}", req.path());
        return Err(AppError::Forbidden);
    };

    let header_matches = actual.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1;
    let cookie_matches = cookie
        .value()
        .as_bytes()
        .ct_eq(expected.as_bytes())
        .unwrap_u8()
        == 1;
    if header_matches && cookie_matches {
        Ok(())
    } else {
        warn!("CSRF token mismatch for {}", req.path());
        Err(AppError::Forbidden)
    }
}

fn require_cookie_csrf(req: &HttpRequest) -> Result<(), AppError> {
    let Some(actual) = csrf_header(req) else {
        warn!("CSRF token missing for {}", req.path());
        return Err(AppError::Forbidden);
    };
    let Some(cookie) = req.cookie("xsrf_token") else {
        warn!("CSRF cookie missing for {}", req.path());
        return Err(AppError::Forbidden);
    };

    if !actual.is_empty()
        && actual
            .as_bytes()
            .ct_eq(cookie.value().as_bytes())
            .unwrap_u8()
            == 1
    {
        Ok(())
    } else {
        warn!("CSRF token mismatch for {}", req.path());
        Err(AppError::Forbidden)
    }
}

#[get("/oauth/start")]
pub async fn oauth_start(
    query: web::Query<OauthStartQuery>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    if !is_allowed_opener_origin(&query.origin, &cfg) {
        return Err(AppError::BadRequest("Invalid opener origin".into()));
    }

    let state = Uuid::new_v4().to_string();
    let url = format!(
        "https://discord.com/oauth2/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(&cfg.discord_redirect_uri),
        urlencoding::encode("email identify guilds"),
        urlencoding::encode(&state),
    );

    let d = cfg.cookie_domain.as_str();
    let mut resp = HttpResponse::Found()
        .append_header((actix_web::http::header::LOCATION, url))
        .finish();
    resp.add_cookie(&oauth_state_cookie(d, &state))?;
    resp.add_cookie(&opener_origin_cookie(d, &query.origin))?;
    Ok(resp)
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
    let cookie_state = req
        .cookie("oauth_state")
        .ok_or_else(|| AppError::BadRequest("Missing oauth_state cookie".into()))?;
    let query_state = query
        .state
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing state parameter".into()))?;
    if cookie_state
        .value()
        .as_bytes()
        .ct_eq(query_state.as_bytes())
        .unwrap_u8()
        != 1
    {
        warn!("OAuth state mismatch on /discord_login");
        return Err(AppError::BadRequest("OAuth state mismatch".into()));
    }

    let data = request_access_token(&cfg, query.code.to_owned(), client.clone()).await?;

    let user = get_user(client.clone(), &data.access_token, &pool).await?;
    let _guilds = get_user_guilds(client, &data.access_token, user.id, &pool).await?;

    let csrf_token = Uuid::new_v4().to_string();

    let (access_token, refresh_token) =
        create_jwt_tokens(user.id, AuthKind::Discord, csrf_token.clone(), &keys).await?;

    let opener_origin = req
        .cookie("opener_origin")
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    let escaped_origin = opener_origin
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('<', "\\u003c");

    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8"><title>Logging in...</title></head>
<body><script>
(function () {{
    var target = "{escaped_origin}";
    if (window.opener && target) {{
        window.opener.postMessage({{ success: 1 }}, target);
    }}
    window.close();
}})();
</script></body></html>"#
    );

    let mut html = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header((
            actix_web::http::header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate, max-age=0",
        ))
        .body(body);

    let d = cfg.cookie_domain.as_str();
    html.add_cookie(&clear_legacy_access_cookie(d))?;
    html.add_cookie(&clear_legacy_refresh_cookie(d))?;
    html.add_cookie(&access_token_cookie(d, &access_token))?;
    html.add_cookie(&refresh_token_cookie(d, &refresh_token))?;
    html.add_cookie(&csrf_cookie(d, &csrf_token))?;
    html.add_cookie(&logged_in_cookie(d))?;
    html.add_cookie(&clear_oauth_state_cookie(d))?;
    html.add_cookie(&clear_opener_origin_cookie(d))?;

    Ok(html)
}

#[cfg(feature = "dev-login")]
#[get("/dev_login")]
pub async fn dev_login(
    req: HttpRequest,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    // Constant time comparison to prevent timing attacks
    let expected = cfg.dev_login_secret.as_deref().ok_or(AppError::Forbidden)?;
    let provided = req
        .headers()
        .get("X-Dev-Login-Secret")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Forbidden)?;
    if expected.as_bytes().ct_eq(provided.as_bytes()).unwrap_u8() != 1 {
        return Err(AppError::Forbidden);
    }

    // If config missing, forbid dev login
    let dev_account_id = cfg.dev_account_id;
    if dev_account_id == 0 {
        return Err(AppError::Forbidden);
    }

    let csrf_token = Uuid::new_v4().to_string();

    let (access_token, refresh_token) =
        create_jwt_tokens(dev_account_id, AuthKind::Dev, csrf_token.clone(), &keys).await?;
    let mut b = NamedFile::open_async("callback.html")
        .await?
        .into_response(&req);

    let d = cfg.cookie_domain.as_str();
    b.add_cookie(&clear_legacy_access_cookie(d))?;
    b.add_cookie(&clear_legacy_refresh_cookie(d))?;
    b.add_cookie(&access_token_cookie(d, &access_token))?;
    b.add_cookie(&refresh_token_cookie(d, &refresh_token))?;
    b.add_cookie(&csrf_cookie(d, &csrf_token))?;
    b.add_cookie(&logged_in_cookie(d))?;

    Ok(b)
}

#[utoipa::path(
    post,
    path = "/api/refresh",
    tag = "auth",
    responses(
        (status = 200, description = "New access token issued", body = RefreshTokenResponse),
        (status = 401, description = "Missing, expired, or invalid refresh token"),
        (status = 403, description = "Missing or invalid CSRF token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("csrf_token" = [])),
)]
#[post("/refresh")]
pub async fn refresh_jwt(
    req: HttpRequest,
    keys: web::Data<AccessKeys>,
    cfg: web::Data<Config>,
) -> Result<impl Responder, AppError> {
    let d = cfg.cookie_domain.as_str();
    let refresh_cookie = match req.cookie("refresh_token") {
        Some(c) => c,
        None => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: missing refresh_token cookie",
                req.path()
            );
            let mut resp = HttpResponse::Unauthorized().finish();
            resp.add_cookie(&clear_access_token_cookie(d))?;
            resp.add_cookie(&clear_refresh_token_cookie(d))?;
            resp.add_cookie(&clear_csrf_cookie(d))?;
            resp.add_cookie(&clear_logged_in_cookie(d))?;
            return Ok(resp);
        }
    };

    let decoded_refresh = match Token::<Refresh>::decode(refresh_cookie.value(), &keys) {
        Ok(token) => token,
        Err(_) => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: expired or invalid refresh token",
                req.path()
            );
            let mut resp = HttpResponse::Unauthorized().json(RefreshTokenError {
                error: "expired_or_invalid_token",
                message: "The refresh token is expired or invalid. Please login again.",
            });
            resp.add_cookie(&clear_access_token_cookie(d))?;
            resp.add_cookie(&clear_refresh_token_cookie(d))?;
            resp.add_cookie(&clear_csrf_cookie(d))?;
            resp.add_cookie(&clear_logged_in_cookie(d))?;
            return Ok(resp);
        }
    };

    require_csrf(&req, &decoded_refresh.csrf)?;

    let csrf_token = Uuid::new_v4().to_string();

    let (new_access_token, new_refresh_token) = create_jwt_tokens(
        decoded_refresh.user_id,
        decoded_refresh.auth_kind,
        csrf_token.clone(),
        &keys,
    )
    .await?;

    let mut response = HttpResponse::Ok().json(RefreshTokenResponse { status: "ok" });
    response.add_cookie(&access_token_cookie(d, &new_access_token))?;
    response.add_cookie(&refresh_token_cookie(d, &new_refresh_token))?;
    response.add_cookie(&csrf_cookie(d, &csrf_token))?;
    response.add_cookie(&logged_in_cookie(d))?;

    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Cookies cleared"),
        (status = 403, description = "Missing or invalid CSRF token", body = crate::errors::ApiError),
    ),
    security(("csrf_token" = [])),
)]
#[post("/logout")]
pub async fn logout(req: HttpRequest, cfg: web::Data<Config>) -> Result<impl Responder, AppError> {
    require_cookie_csrf(&req)?;

    let d = cfg.cookie_domain.as_str();
    let mut resp = HttpResponse::Ok().finish();
    resp.add_cookie(&clear_access_token_cookie(d))?;
    resp.add_cookie(&clear_refresh_token_cookie(d))?;
    resp.add_cookie(&clear_csrf_cookie(d))?;
    resp.add_cookie(&clear_logged_in_cookie(d))?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::{
        is_allowed_opener_origin, require_cookie_csrf, require_csrf, RefreshTokenResponse,
    };
    use crate::config::Config;
    use actix_web::{cookie::Cookie, test as actix_test};

    fn cfg() -> Config {
        Config {
            database_url: "postgres://user:password@localhost/db".into(),
            client_id: "client".into(),
            client_secret: "secret".into(),
            access_secret: "access".into(),
            refresh_secret: "refresh".into(),
            dev_account_id: 1,
            dev_login_secret: Some("dev".into()),
            cors_allowed_origin: "http://localhost:3000".into(),
            oauth_allowed_opener_origins: vec!["http://localhost:3000".into()],
            oauth_allowed_opener_host_suffixes: vec!["patrykstyla.com".into()],
            cookie_domain: "localhost".into(),
            discord_redirect_uri: "http://localhost:8900/api/discord_login".into(),
            grpc_address: "http://[::1]:50052".into(),
            fbi_agent_registry_secret: None,
            host: "127.0.0.1".into(),
            port: 8900,
            db_max_connections: 20,
        }
    }

    #[test]
    fn opener_origin_allows_configured_suffix_and_exact_local_dev() {
        let cfg = cfg();

        assert!(is_allowed_opener_origin("https://patrykstyla.com", &cfg));
        assert!(is_allowed_opener_origin(
            "https://app.patrykstyla.com",
            &cfg
        ));
        assert!(is_allowed_opener_origin("http://localhost:3000", &cfg));
    }

    #[test]
    fn opener_origin_rejects_production_when_not_configured() {
        let mut cfg = cfg();
        cfg.oauth_allowed_opener_host_suffixes.clear();

        assert!(!is_allowed_opener_origin("https://patrykstyla.com", &cfg));
        assert!(!is_allowed_opener_origin(
            "https://app.patrykstyla.com",
            &cfg
        ));
    }

    #[test]
    fn opener_origin_rejects_unrelated_origins() {
        let cfg = cfg();

        assert!(!is_allowed_opener_origin("https://evil.com", &cfg));
        assert!(!is_allowed_opener_origin("http://patrykstyla.com", &cfg));
        assert!(!is_allowed_opener_origin(
            "https://evilpatrykstyla.com",
            &cfg
        ));
        assert!(!is_allowed_opener_origin(
            "https://app.patrykstyla.com/callback",
            &cfg
        ));
    }

    #[test]
    fn csrf_requires_matching_header_cookie_and_expected_token() {
        let req = actix_test::TestRequest::default()
            .insert_header(("X-CSRF-Token", "csrf-123"))
            .cookie(Cookie::new("xsrf_token", "csrf-123"))
            .to_http_request();

        assert!(require_csrf(&req, "csrf-123").is_ok());
        assert!(require_csrf(&req, "csrf-456").is_err());
        assert!(require_cookie_csrf(&req).is_ok());
    }

    #[test]
    fn refresh_response_has_no_token_field() -> Result<(), serde_json::Error> {
        let json = serde_json::to_value(RefreshTokenResponse { status: "ok" })?;

        assert_eq!(json["status"], "ok");
        assert!(json.get("token").is_none());
        Ok(())
    }
}

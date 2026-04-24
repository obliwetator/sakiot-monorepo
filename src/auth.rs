use actix_files::NamedFile;
use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{
    cookie::{Cookie, SameSite},
    get,
    http::Method,
    web, Error, HttpMessage, HttpRequest, HttpResponse, Responder,
};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Postgres};
use time::{Duration, OffsetDateTime};
use tracing::warn;
use uuid::Uuid;

use crate::config::{
    CLIENT_ID, CLIENT_SECRET, COOKIE_DOMAIN, DEV_ACCOUNT_ID, DISCORD_REDIRECT_URI,
};
use crate::errors::AppError;
use crate::user::{get_user, get_user_guilds};

pub struct AccessKeys {
    pub access_encode: EncodingKey,
    pub refresh_encode: EncodingKey,
    pub access_decode: DecodingKey,
    pub refresh_decode: DecodingKey,
}

pub const BASE_URL: &str = "https://discord.com/api/v10/";

pub const BASE_AUTH_URL: &str = "https://discord.com/oauth2/authorize/";
pub const TOKEN_URL: &str = "https://discord.com/api/oauth2/token/";
const JWT_ACCESS_EXPIRY: i64 = 900;
const JWT_REFRESH_EXPIRY: i64 = 7;

// trait Token {
//     fn encode(id: i64, access_token: String, key: &EncodingKey) -> String;
// }

#[derive(Clone)]
pub struct Access;
#[derive(Clone)]
pub struct Refresh;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token<T> {
    #[serde(with = "jwt_numeric_date")]
    pub exp: OffsetDateTime,
    pub id: i64,
    pub token: String,
    pub csrf: String,
    state: std::marker::PhantomData<T>,
}

impl Token<Access> {
    pub fn encode(id: i64, access_token: String, csrf: String, key: &EncodingKey) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::seconds(JWT_ACCESS_EXPIRY);

        let access = Self {
            exp,
            id,
            token: access_token,
            csrf,
            state: std::marker::PhantomData::<Access>,
        };
        encode(&Header::default(), &access, key).map_err(|_| AppError::InternalError)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, AppError> {
        let mut val = Validation::default();
        val.leeway = 0;
        decode::<Self>(token, &keys.access_decode, &val)
            .map(|ok| ok.claims)
            .map_err(|_| AppError::InvalidToken)
    }
}

impl Token<Refresh> {
    pub fn encode(id: i64, refresh_token: String, csrf: String, key: &EncodingKey) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::days(JWT_REFRESH_EXPIRY);

        let refresh = Self {
            exp,
            id,
            token: refresh_token,
            csrf,
            state: std::marker::PhantomData::<Refresh>,
        };
        encode(&Header::default(), &refresh, key).map_err(|_| AppError::InternalError)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, AppError> {
        let mut val = Validation::default();
        val.leeway = 0;
        decode::<Self>(token, &keys.refresh_decode, &val)
            .map(|ok| ok.claims)
            .map_err(|_| AppError::InvalidToken)
    }
}

fn csrf_cookie(value: &str) -> Cookie<'static> {
    Cookie::build("xsrf_token", value.to_string())
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .finish()
}

fn clear_csrf_cookie() -> Cookie<'static> {
    Cookie::build("xsrf_token", "")
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish()
}

fn access_token_cookie(value: &str) -> Cookie<'static> {
    Cookie::build("access_token", value.to_string())
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .finish()
}

fn refresh_token_cookie(value: &str) -> Cookie<'static> {
    Cookie::build("refresh_token", value.to_string())
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .finish()
}

fn clear_access_token_cookie() -> Cookie<'static> {
    Cookie::build("access_token", "")
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish()
}

fn clear_refresh_token_cookie() -> Cookie<'static> {
    Cookie::build("refresh_token", "")
        .domain(COOKIE_DOMAIN.as_str())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish()
}

// Clear legacy cookies stored under Path=/api from pre-fix server versions.
// Same name + different Path = separate browser entries; without this the
// stale ones shadow the new Path=/ cookies on every /api/* request.
fn clear_legacy_access_cookie() -> Cookie<'static> {
    Cookie::build("access_token", "")
        .domain(COOKIE_DOMAIN.as_str())
        .path("/api")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish()
}

fn clear_legacy_refresh_cookie() -> Cookie<'static> {
    Cookie::build("refresh_token", "")
        .domain(COOKIE_DOMAIN.as_str())
        .path("/api")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish()
}

#[derive(Deserialize, Debug)]
pub struct DiscordLoginCode {
    code: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordBotAuthData {
    client_id: &'static str,
    client_secret: &'static str,
    grant_type: &'static str,
    code: String,
    redirect_uri: &'static str,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordBotAuthDataRefresh {
    client_id: &'static str,
    client_secret: &'static str,
    grant_type: &'static str,
    refresh_token: String,
}

impl DiscordBotAuthDataRefresh {
    fn new(refresh_token: String) -> Self {
        Self {
            client_id: CLIENT_ID.as_str(),
            client_secret: CLIENT_SECRET.as_str(),
            grant_type: "refresh_token",
            refresh_token,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DiscordTokenData {
    access_token: String,
    expires_in: i32,
    refresh_token: String,
    scope: String,
    token_type: String,
}

impl Default for DiscordBotAuthData {
    fn default() -> Self {
        Self {
            client_id: CLIENT_ID.as_str(),
            client_secret: CLIENT_SECRET.as_str(),
            grant_type: "authorization_code",
            code: String::from(""),
            redirect_uri: DISCORD_REDIRECT_URI.as_str(),
        }
    }
}

pub async fn request_access_token(
    code: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthData {
        code,
        ..Default::default()
    };

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    let json = result.json::<DiscordTokenData>().await?;

    Ok(json)
}

pub async fn request_refresh_token(
    refresh_token: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthDataRefresh::new(refresh_token);

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    let json = result.json::<DiscordTokenData>().await?;

    Ok(json)
}

#[get("/discord_login")]
pub async fn discord_login(
    req: HttpRequest,
    query: web::Query<DiscordLoginCode>,
    pool: web::Data<Pool<Postgres>>,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
) -> Result<impl Responder, AppError> {
    let data = request_access_token(query.code.to_owned(), client.clone()).await?;

    let user = get_user(client.clone(), &data.access_token, &pool).await?;
    let _guilds = get_user_guilds(client, &data.access_token, user.id, &pool).await?;

    let csrf_token = Uuid::new_v4().to_string();

    let (access_token, refresh_token) =
        create_jwt_tokens(data.access_token, data.refresh_token, user.id, csrf_token.clone(), &keys).await?;
    let mut b = NamedFile::open_async("callback.html")
        .await?
        .into_response(&req);

    b.headers_mut().insert(
        actix_web::http::header::CACHE_CONTROL,
        actix_web::http::header::HeaderValue::from_static(
            "no-store, no-cache, must-revalidate, max-age=0",
        ),
    );

    b.add_cookie(&clear_legacy_access_cookie())
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&clear_legacy_refresh_cookie())
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&access_token_cookie(&access_token))
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&refresh_token_cookie(&refresh_token))
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&csrf_cookie(&csrf_token))
        .map_err(|_| AppError::InternalError)?;

    Ok(b)
}

#[get("/dev_login")]
pub async fn dev_login(
    req: HttpRequest,
    keys: web::Data<AccessKeys>,
) -> Result<impl Responder, AppError> {
    let dev_account_id = DEV_ACCOUNT_ID.parse::<i64>().unwrap_or(0);
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

    b.add_cookie(&clear_legacy_access_cookie())
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&clear_legacy_refresh_cookie())
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&access_token_cookie(&access_token))
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&refresh_token_cookie(&refresh_token))
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&csrf_cookie(&csrf_token))
        .map_err(|_| AppError::InternalError)?;

    Ok(b)
}

#[get("/refresh")]
pub async fn refresh_jwt(
    req: HttpRequest,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
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
        let data = request_refresh_token(decoded_refresh.token, client.clone()).await?;

        create_jwt_tokens(
            data.access_token,
            data.refresh_token,
            decoded_refresh.id,
            csrf_token.clone(),
            &keys,
        )
        .await?
    };

    let mut response = HttpResponse::Ok().json(json!({ "token": new_access_token }));
    response
        .add_cookie(&access_token_cookie(&new_access_token))
        .map_err(|_| AppError::InternalError)?;
    response
        .add_cookie(&refresh_token_cookie(&new_refresh_token))
        .map_err(|_| AppError::InternalError)?;
    response
        .add_cookie(&csrf_cookie(&csrf_token))
        .map_err(|_| AppError::InternalError)?;

    Ok(response)
}

#[get("/logout")]
pub async fn logout() -> Result<impl Responder, AppError> {
    let mut resp = HttpResponse::Ok().finish();
    resp.add_cookie(&clear_access_token_cookie())
        .map_err(|_| AppError::InternalError)?;
    resp.add_cookie(&clear_refresh_token_cookie())
        .map_err(|_| AppError::InternalError)?;
    resp.add_cookie(&clear_csrf_cookie())
        .map_err(|_| AppError::InternalError)?;
    Ok(resp)
}

#[get("/token")]
pub async fn get_token(req: HttpRequest) -> Result<impl Responder, AppError> {
    let access_cookie = req
        .cookie("access_token")
        .ok_or_else(|| AppError::BadRequest("Missing access_token cookie".into()))?;

    let json = json!({ "token": access_cookie.value() });

    Ok(HttpResponse::Ok().json(json))
}

async fn create_jwt_tokens(
    access_token: String,
    refresh_token: String,
    id: i64,
    csrf: String,
    keys: &web::Data<AccessKeys>,
) -> Result<(String, String), AppError> {
    let access_token = Token::<Access>::encode(id, access_token, csrf.clone(), &keys.access_encode)?;

    let refresh_token = Token::<Refresh>::encode(id, refresh_token, csrf, &keys.refresh_encode)?;

    Ok((access_token, refresh_token))
}

pub struct AuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = SayHiMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SayHiMiddleware { service }))
    }
}

pub struct SayHiMiddleware<S> {
    pub service: S,
}

impl<S, B> Service<ServiceRequest> for SayHiMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        tracing::info!("PATH: {:#?}", req.path());
        if req.path() == "/api/discord_login"
            || req.path() == "/api/dev_login"
            || req.path() == "/api/refresh"
            || req.path() == "/api/logout"
        {
            // No auth required for login, refresh, logout
            let res = self.service.call(req);

            Box::pin(async move {
                // forwarded responses map to "left" body
                res.await.map(ServiceResponse::map_into_left_body)
            })
        } else {
            let keys = match req.app_data::<web::Data<AccessKeys>>() {
                Some(k) => k,
                None => {
                    tracing::error!("AccessKeys not in app_data — server misconfigured");
                    let (request, _pl) = req.into_parts();
                    let response = HttpResponse::InternalServerError()
                        .finish()
                        .map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            };

            let access_cookie = match req.cookie("access_token") {
                Some(c) => c,
                None => {
                    warn!(
                        "Unauthorized access attempt to middleware {}: missing access_token cookie",
                        req.path()
                    );
                    let (request, _pl) = req.into_parts();
                    let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            };

            let decoded_access = match Token::<Access>::decode(access_cookie.value(), keys) {
                Ok(token) => token,
                Err(_) => {
                    warn!(
                        "Unauthorized access attempt to middleware {}: invalid or expired token",
                        req.path()
                    );
                    let (request, _pl) = req.into_parts();
                    let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            };

            // CSRF check for state-changing methods
            if req.method() != Method::GET && req.method() != Method::HEAD && req.method() != Method::OPTIONS {
                let csrf_header = req.headers().get("X-CSRF-Token")
                    .and_then(|v| v.to_str().ok());
                if csrf_header != Some(&decoded_access.csrf) {
                    warn!(
                        "CSRF token mismatch for {} (expected present)",
                        req.path()
                    );
                    let (request, _pl) = req.into_parts();
                    let response = HttpResponse::Forbidden()
                        .json(json!({"error": "invalid_csrf_token"}))
                        .map_into_right_body();
                    return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
                }
            }

            req.extensions_mut().insert(decoded_access);
            let res = self.service.call(req);

            Box::pin(async move {
                // forwarded responses map to "left" body
                res.await.map(ServiceResponse::map_into_left_body)
            })
        }
    }
}

mod jwt_numeric_date {
    //! Custom serialization of OffsetDateTime to conform with the JWT spec (RFC 7519 section 2, "Numeric Date")
    use serde::{self, Deserialize, Deserializer, Serializer};
    use time::OffsetDateTime;

    /// Serializes an OffsetDateTime to a Unix timestamp (milliseconds since 1970/1/1T00:00:00T)
    pub fn serialize<S>(date: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let timestamp = date.unix_timestamp();
        serializer.serialize_i64(timestamp)
    }

    /// Attempts to deserialize an i64 and use as a Unix timestamp
    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        OffsetDateTime::from_unix_timestamp(i64::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("invalid Unix timestamp value"))
    }
}

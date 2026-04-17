#![allow(dead_code)]
use actix_files::NamedFile;
use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{
    cookie::{Cookie, SameSite},
    get, web, Error, HttpMessage, HttpRequest, HttpResponse, Responder,
};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Postgres};
use time::{Duration, OffsetDateTime};
use tracing::warn;

use crate::config::{CLIENT_ID, CLIENT_SECRET};
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
const JWT_ACCESS_EXPIRY: i64 = 100;
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
    // aud: String,         // Optional. Audience
    #[serde(with = "jwt_numeric_date")]
    pub exp: OffsetDateTime, // Required (validate_exp defaults to true in validation). Expiration time (as UTC timestamp)
    pub id: i64,
    pub token: String,
    // iat: usize,          // Optional. Issued at (as UTC timestamp)
    // iss: String,         // Optional. Issuer
    // nbf: usize,          // Optional. Not Before (as UTC timestamp)
    // sub: String,         // Optional. Subject (whom token refers to)
    state: std::marker::PhantomData<T>,
}

impl Token<Access> {
    pub fn encode(id: i64, access_token: String, key: &EncodingKey) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::seconds(JWT_ACCESS_EXPIRY);

        let access = Self {
            exp,
            id,
            token: access_token,
            state: std::marker::PhantomData::<Access>,
        };
        encode(&Header::default(), &access, key).map_err(|_| AppError::InternalError)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, jsonwebtoken::errors::Error> {
        let mut val = Validation::default();
        val.leeway = 0;
        match decode::<Self>(token, &keys.access_decode, &val) {
            Ok(ok) => Ok(ok.claims),
            Err(err) => Err(err),
        }
    }
}

impl Token<Refresh> {
    pub fn encode(id: i64, refresh_token: String, key: &EncodingKey) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::days(JWT_REFRESH_EXPIRY);

        let refresh = Self {
            exp,
            id,
            token: refresh_token,
            state: std::marker::PhantomData::<Refresh>,
        };
        encode(&Header::default(), &refresh, key).map_err(|_| AppError::InternalError)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, jsonwebtoken::errors::Error> {
        let mut val = Validation::default();
        val.leeway = 0;
        match decode::<Self>(token, &keys.refresh_decode, &val) {
            Ok(ok) => Ok(ok.claims),
            Err(err) => Err(err),
        }
    }
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
            redirect_uri: "https://dev.patrykstyla.com/api/discord_login",
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

    let (access_token, refresh_token) =
        create_jwt_tokens(data.access_token, data.refresh_token, user.id, &keys).await?;
    let mut b = NamedFile::open_async("callback.html")
        .await?
        .into_response(&req);

    let access_token_cookie = Cookie::build("access_token", access_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .finish();

    let refresh_token_cookie = Cookie::build("refresh_token", refresh_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .finish();

    // Clear legacy cookies stored under Path=/api from pre-fix server versions.
    // Same name + different Path = separate browser entries; without this the
    // stale ones shadow the new Path=/ cookies on every /api/* request.
    let clear_old_access = Cookie::build("access_token", "")
        .domain(".patrykstyla.com")
        .path("/api")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish();
    let clear_old_refresh = Cookie::build("refresh_token", "")
        .domain(".patrykstyla.com")
        .path("/api")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish();

    b.add_cookie(&clear_old_access)
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&clear_old_refresh)
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&access_token_cookie)
        .map_err(|_| AppError::InternalError)?;
    b.add_cookie(&refresh_token_cookie)
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

    let data = request_refresh_token(decoded_refresh.token, client.clone()).await?;

    let (new_access_token, new_refresh_token) = create_jwt_tokens(
        data.access_token,
        data.refresh_token,
        decoded_refresh.id,
        &keys,
    )
    .await?;

    let access_token_cookie = Cookie::build("access_token", new_access_token.clone())
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .finish();

    let refresh_token_cookie = Cookie::build("refresh_token", new_refresh_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .finish();

    let mut response = HttpResponse::Ok().json(json!({ "token": new_access_token }));
    response
        .add_cookie(&access_token_cookie)
        .map_err(|_| AppError::InternalError)?;
    response
        .add_cookie(&refresh_token_cookie)
        .map_err(|_| AppError::InternalError)?;

    Ok(response)
}

#[get("/logout")]
pub async fn logout() -> Result<impl Responder, AppError> {
    let clear_access = Cookie::build("access_token", "")
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish();
    let clear_refresh = Cookie::build("refresh_token", "")
        .domain(".patrykstyla.com")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish();

    let mut resp = HttpResponse::Ok().finish();
    resp.add_cookie(&clear_access)
        .map_err(|_| AppError::InternalError)?;
    resp.add_cookie(&clear_refresh)
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
    keys: &web::Data<AccessKeys>,
) -> Result<(String, String), AppError> {
    let access_token = Token::<Access>::encode(id, access_token, &keys.access_encode)?;

    let refresh_token = Token::<Refresh>::encode(id, refresh_token, &keys.refresh_encode)?;

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
            || req.path() == "/api/refresh"
            || req.path() == "/api/logout"
            || req.path() == "/api/dashboard/stream"
        {
            // Dont validate the token if user is trying to login, refresh, or connecting to websocket
            let res = self.service.call(req);

            Box::pin(async move {
                // forwarded responses map to "left" body
                res.await.map(ServiceResponse::map_into_left_body)
            })
        } else {
            let keys = req
                .app_data::<web::Data<AccessKeys>>()
                .expect("AccessKeys not in app_data");

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
            tracing::info!("Decoded access token: {:#?}", decoded_access.exp);

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

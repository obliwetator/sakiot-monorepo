#![allow(dead_code)]
use actix_files::NamedFile;
use actix_web::{cookie::Cookie, get, web, HttpRequest, HttpResponse, Responder};
use jsonwebtoken::{decode, encode, EncodingKey, Header, Validation};
use reqwest::Client;
use serde::{de::value, Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Postgres};
use time::{Duration, OffsetDateTime};
use tracing::warn;

pub use crate::get_access_and_refresh_tokens;
use crate::{
    secrets::{CLIENT_ID, CLIENT_SECRET},
    user::{get_user, get_user_guilds},
    AccessKeys,
};

pub const BASE_URL: &str = "https://discord.com/api/v10/";

pub const BASE_AUTH_URL: &str = "https://discord.com/oauth2/authorize/";
pub const TOKEN_URL: &str = "https://discord.com/api/oauth2/token/";
const JWT_ACCESS_EXPIRY: i64 = 10;
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
    pub fn encode(id: i64, access_token: String, key: &EncodingKey) -> String {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::seconds(JWT_ACCESS_EXPIRY);

        let access = Self {
            exp,
            id,
            token: access_token,
            state: std::marker::PhantomData::<Access>,
        };
        encode(&Header::default(), &access, key).unwrap()
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
    pub fn encode(id: i64, refresh_token: String, key: &EncodingKey) -> String {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::days(JWT_REFRESH_EXPIRY);

        let refresh = Self {
            exp,
            id,
            token: refresh_token,
            state: std::marker::PhantomData::<Refresh>,
        };
        encode(&Header::default(), &refresh, key).unwrap()
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
            client_id: CLIENT_ID,
            client_secret: CLIENT_SECRET,
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
            client_id: CLIENT_ID,
            client_secret: CLIENT_SECRET,
            grant_type: "authorization_code",
            code: String::from(""),
            redirect_uri: "https://dev.patrykstyla.com/api/discord_login",
        }
    }
}

pub async fn request_access_token(code: String, client: web::Data<Client>) -> DiscordTokenData {
    let data = DiscordBotAuthData {
        code,
        ..Default::default()
    };

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await
        .unwrap();

    // let text = result.text().await.unwrap();
    let json = result.json::<DiscordTokenData>().await.unwrap();

    json
}

pub async fn request_refresh_token(
    refresh_token: String,
    client: web::Data<Client>,
) -> DiscordTokenData {
    let data = DiscordBotAuthDataRefresh::new(refresh_token);

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await
        .unwrap();

    // let text = result.text().await.unwrap();
    let json = result.json::<DiscordTokenData>().await.unwrap();

    json
}

#[get("/discord_login")]
pub async fn discord_login(
    req: HttpRequest,
    query: web::Query<DiscordLoginCode>,
    pool: web::Data<Pool<Postgres>>,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
) -> impl Responder {
    let data = request_access_token(query.code.to_owned(), client.clone()).await;

    let user = get_user(client.clone(), &data.access_token, &pool).await;
    let _guilds = get_user_guilds(client, &data.access_token, user.id, &pool).await;

    // let guilds_id: Vec<i64> = guilds.iter().map(|a| a.id).collect();

    let (access_token, refresh_token) =
        create_jwt_tokens(data.access_token, data.refresh_token, user.id, &keys).await;
    let mut b = NamedFile::open_async("callback.html")
        .await
        .unwrap()
        .into_response(&req);

    let access_token_cookie = Cookie::build("access_token", access_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .secure(true)
        .http_only(true)
        .finish();

    let refresh_token_cookie = Cookie::build("refresh_token", refresh_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .secure(true)
        .http_only(true)
        .finish();

    b.add_cookie(&access_token_cookie).unwrap();
    b.add_cookie(&refresh_token_cookie).unwrap();

    b
}

#[get("/refresh")]
pub async fn refresh_jwt(
    req: HttpRequest,
    client: web::Data<Client>,
    keys: web::Data<AccessKeys>,
) -> impl Responder {
    let headers = req.headers();
    let cookie = match headers.get("cookie") {
        Some(cookie) => cookie,
        None => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: missing cookie",
                req.path()
            );
            return HttpResponse::Unauthorized().finish();
        }
    };

    let (_, refresh_token) = get_access_and_refresh_tokens(cookie);

    let decoded_refresh = match Token::<Refresh>::decode(refresh_token, &keys) {
        Ok(token) => token,
        Err(_) => {
            warn!(
                "Unauthorized access attempt to refresh_jwt{}: expired or invalid refresh token",
                req.path()
            );
            return HttpResponse::Unauthorized().json(json!({
                "error": "expired_or_invalid_token",
                "message": "The refresh token is expired or invalid. Please login again."
            }));
        }
    };

    let data = request_refresh_token(decoded_refresh.token, client.clone()).await;

    let (new_access_token, new_refresh_token) = create_jwt_tokens(
        data.access_token,
        data.refresh_token,
        decoded_refresh.id,
        &keys,
    )
    .await;

    let access_token_cookie = Cookie::build("access_token", new_access_token.clone())
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .secure(true)
        .http_only(true)
        .finish();

    let refresh_token_cookie = Cookie::build("refresh_token", new_refresh_token)
        .max_age(actix_web::cookie::time::Duration::days(7))
        .domain(".patrykstyla.com")
        .secure(true)
        .http_only(true)
        .finish();

    let mut response = HttpResponse::Ok().json(json!({ "token": new_access_token }));
    response.add_cookie(&access_token_cookie).unwrap();
    response.add_cookie(&refresh_token_cookie).unwrap();

    response
}

#[get("/token")]
pub async fn get_token(req: HttpRequest) -> impl Responder {
    let headers = req.headers();
    let cookie = match headers.get("cookie") {
        Some(cookie) => cookie,
        None => {
            panic!("");
        }
    };

    let (access_token, _) = get_access_and_refresh_tokens(cookie);

    let json = json!({ "token": access_token });

    HttpResponse::Ok().json(json)
}

async fn create_jwt_tokens(
    access_token: String,
    refresh_token: String,
    id: i64,
    keys: &web::Data<AccessKeys>,
) -> (String, String) {
    let access_token = Token::<Access>::encode(id, access_token, &keys.access_encode);

    let refresh_token = Token::<Refresh>::encode(id, refresh_token, &keys.refresh_encode);

    (access_token, refresh_token)
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

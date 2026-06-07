use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::errors::AppError;

const JWT_ACCESS_EXPIRY_SECS: i64 = 900;
pub const JWT_REFRESH_EXPIRY_DAYS: i64 = 7;

pub struct AccessKeys {
    pub access_encode: EncodingKey,
    pub refresh_encode: EncodingKey,
    pub access_decode: DecodingKey,
    pub refresh_decode: DecodingKey,
}

#[derive(Clone)]
pub struct Access;
#[derive(Clone)]
pub struct Refresh;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Discord,
    Dev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TokenPurpose {
    Access,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token<T> {
    #[serde(with = "jwt_numeric_date")]
    pub exp: OffsetDateTime,
    pub user_id: i64,
    pub csrf: String,
    pub auth_kind: AuthKind,
    purpose: TokenPurpose,
    state: std::marker::PhantomData<T>,
}

fn validation() -> Validation {
    let mut val = Validation::new(Algorithm::HS256);
    val.leeway = 0;
    val.set_required_spec_claims(&["exp"]);
    val
}

impl Token<Access> {
    pub fn encode(
        id: i64,
        auth_kind: AuthKind,
        csrf: String,
        key: &EncodingKey,
    ) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::seconds(JWT_ACCESS_EXPIRY_SECS);

        let access = Self {
            exp,
            user_id: id,
            csrf,
            auth_kind,
            purpose: TokenPurpose::Access,
            state: std::marker::PhantomData::<Access>,
        };
        Ok(encode(&Header::default(), &access, key)?)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, AppError> {
        decode::<Self>(token, &keys.access_decode, &validation())
            .map(|ok| ok.claims)
            .and_then(|claims| {
                if claims.purpose == TokenPurpose::Access {
                    Ok(claims)
                } else {
                    Err(jsonwebtoken::errors::Error::from(
                        jsonwebtoken::errors::ErrorKind::InvalidToken,
                    ))
                }
            })
            .map_err(|e| {
                tracing::debug!(?e, "access token decode failed");
                AppError::InvalidToken
            })
    }
}

impl Token<Refresh> {
    pub fn encode(
        id: i64,
        auth_kind: AuthKind,
        csrf: String,
        key: &EncodingKey,
    ) -> Result<String, AppError> {
        let iat = OffsetDateTime::now_utc();
        let exp = iat + Duration::days(JWT_REFRESH_EXPIRY_DAYS);

        let refresh = Self {
            exp,
            user_id: id,
            csrf,
            auth_kind,
            purpose: TokenPurpose::Refresh,
            state: std::marker::PhantomData::<Refresh>,
        };
        Ok(encode(&Header::default(), &refresh, key)?)
    }
    pub fn decode(token: &str, keys: &AccessKeys) -> Result<Self, AppError> {
        decode::<Self>(token, &keys.refresh_decode, &validation())
            .map(|ok| ok.claims)
            .and_then(|claims| {
                if claims.purpose == TokenPurpose::Refresh {
                    Ok(claims)
                } else {
                    Err(jsonwebtoken::errors::Error::from(
                        jsonwebtoken::errors::ErrorKind::InvalidToken,
                    ))
                }
            })
            .map_err(|e| {
                tracing::debug!(?e, "refresh token decode failed");
                AppError::InvalidToken
            })
    }
}

mod jwt_numeric_date {
    //! Custom serialization of OffsetDateTime to conform with the JWT spec (RFC 7519 section 2, "Numeric Date")
    use serde::{self, Deserialize, Deserializer, Serializer};
    use time::OffsetDateTime;

    pub fn serialize<S>(date: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(date.unix_timestamp())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        OffsetDateTime::from_unix_timestamp(i64::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("invalid Unix timestamp value"))
    }
}

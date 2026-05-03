use std::env;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    Missing(&'static str),
    #[error("env {key} is not a valid {ty}: {value}")]
    Parse {
        key: &'static str,
        ty: &'static str,
        value: String,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub access_secret: String,
    pub refresh_secret: String,
    pub dev_account_id: i64,
    pub dev_login_secret: Option<String>,
    pub cors_allowed_origin: String,
    pub cookie_domain: String,
    pub discord_redirect_uri: String,
    pub grpc_address: String,
    pub host: String,
    pub port: u16,
    pub db_max_connections: u32,
}

fn require(key: &'static str) -> Result<String, ConfigError> {
    env::var(key).map_err(|_| ConfigError::Missing(key))
}

fn optional(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.into())
}

fn parse<T: FromStr>(key: &'static str, default: T) -> Result<T, ConfigError> {
    match env::var(key) {
        Ok(v) => v.parse().map_err(|_| ConfigError::Parse {
            key,
            ty: std::any::type_name::<T>(),
            value: v,
        }),
        Err(_) => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: require("DATABASE_URL")?,
            client_id: require("DISCORD_CLIENT_ID")?,
            client_secret: require("DISCORD_CLIENT_SECRET")?,
            access_secret: require("JWT_ACCESS_SECRET")?,
            refresh_secret: require("JWT_REFRESH_SECRET")?,
            dev_account_id: parse("DEV_ACCOUNT_ID", 0)?,
            dev_login_secret: env::var("DEV_LOGIN_SECRET").ok().filter(|s| !s.is_empty()),
            cors_allowed_origin: optional("CORS_ALLOWED_ORIGIN", "http://localhost:3000"),
            cookie_domain: optional("COOKIE_DOMAIN", "localhost"),
            discord_redirect_uri: optional(
                "DISCORD_REDIRECT_URI",
                "http://localhost:8900/api/discord_login",
            ),
            grpc_address: optional("GRPC_ADDRESS", "http://[::1]:50052"),
            host: optional("HOST", "127.0.0.1"),
            port: parse("PORT", 8900u16)?,
            db_max_connections: parse("DB_MAX_CONNECTIONS", 20u32)?,
        })
    }
}

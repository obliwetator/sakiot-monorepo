use once_cell::sync::Lazy;
use std::env;

fn load(key: &str) -> String {
    #[allow(clippy::print_stderr)]
    env::var(key).unwrap_or_else(|_| {
        eprintln!("Fatal: missing required environment variable: {key}");
        std::process::exit(1);
    })
}

pub static DATABASE_URL: Lazy<String> = Lazy::new(|| load("DATABASE_URL"));
pub static CLIENT_ID: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_ID"));
pub static CLIENT_SECRET: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_SECRET"));
pub static ACCESS_SECRET: Lazy<String> = Lazy::new(|| load("JWT_ACCESS_SECRET"));
pub static REFRESH_SECRET: Lazy<String> = Lazy::new(|| load("JWT_REFRESH_SECRET"));
pub static DEV_ACCOUNT_ID: Lazy<String> =
    Lazy::new(|| env::var("DEV_ACCOUNT_ID").unwrap_or_else(|_| "".into()));
pub static CORS_ALLOWED_ORIGIN: Lazy<String> =
    Lazy::new(|| env::var("CORS_ALLOWED_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".into()));
pub static COOKIE_DOMAIN: Lazy<String> =
    Lazy::new(|| env::var("COOKIE_DOMAIN").unwrap_or_else(|_| "localhost".into()));
pub static DISCORD_REDIRECT_URI: Lazy<String> =
    Lazy::new(|| env::var("DISCORD_REDIRECT_URI").unwrap_or_else(|_| "http://localhost:8900/api/discord_login".into()));
pub static GRPC_ADDRESS: Lazy<String> =
    Lazy::new(|| env::var("GRPC_ADDRESS").unwrap_or_else(|_| "http://[::1]:50052".into()));

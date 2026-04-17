use once_cell::sync::Lazy;
use std::env;

fn load(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("missing env var: {key}"))
}

pub static DATABASE_URL: Lazy<String> = Lazy::new(|| load("DATABASE_URL"));
pub static CLIENT_ID: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_ID"));
pub static CLIENT_SECRET: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_SECRET"));
pub static ACCESS_SECRET: Lazy<String> = Lazy::new(|| load("JWT_ACCESS_SECRET"));
pub static REFRESH_SECRET: Lazy<String> = Lazy::new(|| load("JWT_REFRESH_SECRET"));
pub static CORS_ALLOWED_ORIGIN: Lazy<String> =
    Lazy::new(|| env::var("CORS_ALLOWED_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".into()));

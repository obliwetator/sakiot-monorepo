use once_cell::sync::Lazy;
use std::env;
use std::str::FromStr;

fn load(key: &str) -> String {
    #[allow(clippy::print_stderr)]
    env::var(key).unwrap_or_else(|_| {
        eprintln!("Fatal: missing required environment variable: {key}");
        std::process::exit(1);
    })
}

fn load_parsed<T: FromStr>(key: &str, default: T) -> T {
    match env::var(key) {
        Ok(v) => v.parse().unwrap_or_else(|_| {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("Fatal: env {key} is not a valid {}", std::any::type_name::<T>());
            }
            std::process::exit(1);
        }),
        Err(_) => default,
    }
}

pub static DATABASE_URL: Lazy<String> = Lazy::new(|| load("DATABASE_URL"));
pub static CLIENT_ID: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_ID"));
pub static CLIENT_SECRET: Lazy<String> = Lazy::new(|| load("DISCORD_CLIENT_SECRET"));
pub static ACCESS_SECRET: Lazy<String> = Lazy::new(|| load("JWT_ACCESS_SECRET"));
pub static REFRESH_SECRET: Lazy<String> = Lazy::new(|| load("JWT_REFRESH_SECRET"));
pub static DEV_ACCOUNT_ID: Lazy<i64> = Lazy::new(|| load_parsed("DEV_ACCOUNT_ID", 0));
pub static CORS_ALLOWED_ORIGIN: Lazy<String> =
    Lazy::new(|| env::var("CORS_ALLOWED_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".into()));
pub static COOKIE_DOMAIN: Lazy<String> =
    Lazy::new(|| env::var("COOKIE_DOMAIN").unwrap_or_else(|_| "localhost".into()));
pub static DISCORD_REDIRECT_URI: Lazy<String> =
    Lazy::new(|| env::var("DISCORD_REDIRECT_URI").unwrap_or_else(|_| "http://localhost:8900/api/discord_login".into()));
pub static GRPC_ADDRESS: Lazy<String> =
    Lazy::new(|| env::var("GRPC_ADDRESS").unwrap_or_else(|_| "http://[::1]:50052".into()));
pub static HOST: Lazy<String> =
    Lazy::new(|| env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()));
pub static PORT: Lazy<u16> = Lazy::new(|| load_parsed("PORT", 8900));
pub static DB_MAX_CONNECTIONS: Lazy<u32> = Lazy::new(|| load_parsed("DB_MAX_CONNECTIONS", 20));

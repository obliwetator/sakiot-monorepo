pub mod cookies;
pub mod device;
pub mod discord;
pub mod handlers;
pub mod jwt;
pub mod middleware;
pub mod sessions;

pub use discord::{BASE_URL, request_access_token, request_refresh_token};
#[cfg(feature = "dev-login")]
pub use handlers::dev_login;
pub use handlers::{
    discord_login, end_all_sessions, end_session, list_sessions, logout, oauth_start, refresh_jwt,
};
pub use jwt::{Access, AccessKeys, AuthKind, Refresh, Token};
pub use middleware::AuthMiddleware;

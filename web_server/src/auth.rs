pub mod cookies;
pub mod discord;
pub mod handlers;
pub mod jwt;
pub mod middleware;

pub use discord::{BASE_URL, request_access_token, request_refresh_token};
#[cfg(feature = "dev-login")]
pub use handlers::dev_login;
pub use handlers::{discord_login, logout, oauth_start, refresh_jwt};
pub use jwt::{Access, AccessKeys, AuthKind, Refresh, Token};
pub use middleware::AuthMiddleware;

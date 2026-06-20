use actix_web::cookie::{Cookie, SameSite, time::Duration};

use super::jwt::JWT_REFRESH_EXPIRY_DAYS;

pub const ACCESS_TOKEN_COOKIE: &str = "__Host-sakiot-access_token";
pub const REFRESH_TOKEN_COOKIE: &str = "__Host-sakiot-refresh_token";
pub const CSRF_COOKIE: &str = "__Host-sakiot-xsrf_token";
pub const LOGGED_IN_COOKIE: &str = "__Host-sakiot-logged_in";
pub const OAUTH_STATE_COOKIE: &str = "__Host-sakiot-oauth_state";

pub fn csrf_cookie(value: &str) -> Cookie<'static> {
    Cookie::build(CSRF_COOKIE, value.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::days(JWT_REFRESH_EXPIRY_DAYS))
        .finish()
}

pub fn clear_csrf_cookie() -> Cookie<'static> {
    Cookie::build(CSRF_COOKIE, "")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn access_token_cookie(value: &str) -> Cookie<'static> {
    Cookie::build(ACCESS_TOKEN_COOKIE, value.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::days(JWT_REFRESH_EXPIRY_DAYS))
        .finish()
}

pub fn refresh_token_cookie(value: &str) -> Cookie<'static> {
    Cookie::build(REFRESH_TOKEN_COOKIE, value.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::days(JWT_REFRESH_EXPIRY_DAYS))
        .finish()
}

pub fn clear_access_token_cookie() -> Cookie<'static> {
    Cookie::build(ACCESS_TOKEN_COOKIE, "")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn clear_refresh_token_cookie() -> Cookie<'static> {
    Cookie::build(REFRESH_TOKEN_COOKIE, "")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn logged_in_cookie() -> Cookie<'static> {
    Cookie::build(LOGGED_IN_COOKIE, "1")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::days(JWT_REFRESH_EXPIRY_DAYS))
        .finish()
}

pub fn clear_logged_in_cookie() -> Cookie<'static> {
    Cookie::build(LOGGED_IN_COOKIE, "")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn oauth_state_cookie(value: &str) -> Cookie<'static> {
    Cookie::build(OAUTH_STATE_COOKIE, value.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::minutes(10))
        .finish()
}

pub fn clear_oauth_state_cookie() -> Cookie<'static> {
    Cookie::build(OAUTH_STATE_COOKIE, "")
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn opener_origin_cookie(domain: &str, value: &str) -> Cookie<'static> {
    Cookie::build("opener_origin", value.to_string())
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::minutes(10))
        .finish()
}

pub fn clear_opener_origin_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("opener_origin", "")
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::seconds(0))
        .finish()
}

// Clear legacy cookies stored under Path=/api from pre-fix server versions.
// Same name + different Path = separate browser entries; without this the
// stale ones shadow the new Path=/ cookies on every /api/* request.
pub fn clear_legacy_access_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("access_token", "")
        .domain(domain.to_string())
        .path("/api")
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn clear_legacy_refresh_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("refresh_token", "")
        .domain(domain.to_string())
        .path("/api")
        .max_age(Duration::seconds(0))
        .finish()
}

#[cfg(test)]
mod tests {
    use super::{ACCESS_TOKEN_COOKIE, access_token_cookie};

    #[test]
    fn auth_cookies_are_host_only() {
        let cookie = access_token_cookie("token");

        assert_eq!(cookie.name(), ACCESS_TOKEN_COOKIE);
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.domain(), None);
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.http_only(), Some(true));
    }
}

use actix_web::cookie::{Cookie, SameSite, time::Duration};

use super::jwt::JWT_REFRESH_EXPIRY_DAYS;

pub const ACCESS_TOKEN_COOKIE: &str = "__Host-sakiot-access_token";
pub const REFRESH_TOKEN_COOKIE: &str = "__Host-sakiot-refresh_token";
pub const CSRF_COOKIE: &str = "__Host-sakiot-xsrf_token";
pub const LOGGED_IN_COOKIE: &str = "__Host-sakiot-logged_in";
pub const OAUTH_STATE_COOKIE: &str = "__Host-sakiot-oauth_state";

// All auth cookies are host-only (`__Host-` prefix: Secure, Path=/, no
// Domain). A zero max_age clears the cookie.
fn host_cookie(
    name: &'static str,
    value: String,
    http_only: bool,
    max_age: Duration,
) -> Cookie<'static> {
    Cookie::build(name, value)
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(http_only)
        .max_age(max_age)
        .finish()
}

const SESSION_LIFETIME: Duration = Duration::days(JWT_REFRESH_EXPIRY_DAYS);
const CLEAR: Duration = Duration::seconds(0);

pub fn csrf_cookie(value: &str) -> Cookie<'static> {
    host_cookie(CSRF_COOKIE, value.to_string(), false, SESSION_LIFETIME)
}

pub fn clear_csrf_cookie() -> Cookie<'static> {
    host_cookie(CSRF_COOKIE, String::new(), false, CLEAR)
}

pub fn access_token_cookie(value: &str) -> Cookie<'static> {
    host_cookie(
        ACCESS_TOKEN_COOKIE,
        value.to_string(),
        true,
        SESSION_LIFETIME,
    )
}

pub fn refresh_token_cookie(value: &str) -> Cookie<'static> {
    host_cookie(
        REFRESH_TOKEN_COOKIE,
        value.to_string(),
        true,
        SESSION_LIFETIME,
    )
}

pub fn clear_access_token_cookie() -> Cookie<'static> {
    host_cookie(ACCESS_TOKEN_COOKIE, String::new(), true, CLEAR)
}

pub fn clear_refresh_token_cookie() -> Cookie<'static> {
    host_cookie(REFRESH_TOKEN_COOKIE, String::new(), true, CLEAR)
}

pub fn logged_in_cookie() -> Cookie<'static> {
    host_cookie(LOGGED_IN_COOKIE, "1".to_string(), false, SESSION_LIFETIME)
}

pub fn clear_logged_in_cookie() -> Cookie<'static> {
    host_cookie(LOGGED_IN_COOKIE, String::new(), false, CLEAR)
}

pub fn oauth_state_cookie(value: &str) -> Cookie<'static> {
    host_cookie(
        OAUTH_STATE_COOKIE,
        value.to_string(),
        true,
        Duration::minutes(10),
    )
}

pub fn clear_oauth_state_cookie() -> Cookie<'static> {
    host_cookie(OAUTH_STATE_COOKIE, String::new(), true, CLEAR)
}

// The opener-origin cookie is intentionally domain-scoped (OAuth popup and
// opener may live on different subdomains) and JS-readable.
fn opener_origin(domain: &str, value: &str, max_age: Duration) -> Cookie<'static> {
    Cookie::build("opener_origin", value.to_string())
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(max_age)
        .finish()
}

pub fn opener_origin_cookie(domain: &str, value: &str) -> Cookie<'static> {
    opener_origin(domain, value, Duration::minutes(10))
}

pub fn clear_opener_origin_cookie(domain: &str) -> Cookie<'static> {
    opener_origin(domain, "", CLEAR)
}

// Clear legacy cookies stored under Path=/api from pre-fix server versions.
// Same name + different Path = separate browser entries; without this the
// stale ones shadow the new Path=/ cookies on every /api/* request.
fn clear_legacy_api_cookie(name: &'static str, domain: &str) -> Cookie<'static> {
    Cookie::build(name, "")
        .domain(domain.to_string())
        .path("/api")
        .max_age(CLEAR)
        .finish()
}

pub fn clear_legacy_access_cookie(domain: &str) -> Cookie<'static> {
    clear_legacy_api_cookie("access_token", domain)
}

pub fn clear_legacy_refresh_cookie(domain: &str) -> Cookie<'static> {
    clear_legacy_api_cookie("refresh_token", domain)
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

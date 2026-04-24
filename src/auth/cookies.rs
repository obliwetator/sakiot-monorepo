use actix_web::cookie::{time::Duration, Cookie, SameSite};

pub fn csrf_cookie(domain: &str, value: &str) -> Cookie<'static> {
    Cookie::build("xsrf_token", value.to_string())
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(false)
        .max_age(Duration::days(7))
        .finish()
}

pub fn clear_csrf_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("xsrf_token", "")
        .domain(domain.to_string())
        .path("/")
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn access_token_cookie(domain: &str, value: &str) -> Cookie<'static> {
    Cookie::build("access_token", value.to_string())
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::days(7))
        .finish()
}

pub fn refresh_token_cookie(domain: &str, value: &str) -> Cookie<'static> {
    Cookie::build("refresh_token", value.to_string())
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::days(7))
        .finish()
}

pub fn clear_access_token_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("access_token", "")
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
        .max_age(Duration::seconds(0))
        .finish()
}

pub fn clear_refresh_token_cookie(domain: &str) -> Cookie<'static> {
    Cookie::build("refresh_token", "")
        .domain(domain.to_string())
        .path("/")
        .same_site(SameSite::Lax)
        .secure(true)
        .http_only(true)
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

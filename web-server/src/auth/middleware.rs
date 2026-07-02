use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{
    Error, HttpMessage, HttpResponse,
    http::{
        Method,
        header::{HeaderName, HeaderValue},
    },
    web,
};
use futures_util::future::{LocalBoxFuture, Ready, ready};
use serde_json::json;
use tracing::warn;

use super::cookies::ACCESS_TOKEN_COOKIE;
use super::jwt::{Access, AccessKeys, Token};

pub struct AuthMiddleware;

fn is_public_api_path(path: &str) -> bool {
    path == "/api/discord_login"
        || path == "/api/oauth/start"
        || (cfg!(feature = "dev-login") && path == "/api/dev_login")
        || path == "/api/refresh"
        || path == "/api/logout"
}

fn warn_unauthorized_middleware_access(_path: &str, _reason: &str) {
    // warn!(
    //     "Unauthorized access attempt to middleware {}: {}",
    //     path, reason
    // );
}

fn latest_access_token(req: &ServiceRequest, keys: &AccessKeys) -> Option<Token<Access>> {
    // Browser order is not an authentication boundary; use the newest token
    // that validates with this environment's signing key.
    req.cookies().ok().and_then(|cookies| {
        cookies
            .iter()
            .filter(|cookie| cookie.name() == ACCESS_TOKEN_COOKIE)
            .filter_map(|cookie| Token::<Access>::decode(cookie.value(), keys).ok())
            .max_by_key(|token| token.exp)
    })
}

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService { service }))
    }
}

pub struct AuthMiddlewareService<S> {
    pub service: S,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        if is_public_api_path(req.path()) {
            let res = self.service.call(req);
            return Box::pin(async move { res.await.map(ServiceResponse::map_into_left_body) });
        }

        let keys = match req.app_data::<web::Data<AccessKeys>>() {
            Some(k) => k,
            None => {
                tracing::error!("AccessKeys not in app_data — server misconfigured");
                let (request, _pl) = req.into_parts();
                let response = HttpResponse::InternalServerError()
                    .finish()
                    .map_into_right_body();
                return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
            }
        };

        let decoded_access = match latest_access_token(&req, keys) {
            Some(token) => token,
            None => {
                let reason = if req.cookie(ACCESS_TOKEN_COOKIE).is_some() {
                    "invalid or expired token"
                } else {
                    "missing access_token cookie"
                };
                warn_unauthorized_middleware_access(req.path(), reason);
                let (request, _pl) = req.into_parts();
                let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
            }
        };

        // CSRF check for state-changing methods
        if req.method() != Method::GET
            && req.method() != Method::HEAD
            && req.method() != Method::OPTIONS
        {
            let csrf_header = req
                .headers()
                .get("X-CSRF-Token")
                .and_then(|v| v.to_str().ok());
            if csrf_header != Some(&decoded_access.csrf) {
                warn!("CSRF token mismatch for {} (expected present)", req.path());
                let (request, _pl) = req.into_parts();
                let response = HttpResponse::Forbidden()
                    .json(json!({"error": "invalid_csrf_token"}))
                    .map_into_right_body();
                return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
            }
        }

        let csrf = decoded_access.csrf.clone();
        req.extensions_mut().insert(decoded_access);
        let res = self.service.call(req);
        Box::pin(async move {
            let mut response = res.await?.map_into_left_body();
            if let Ok(value) = HeaderValue::from_str(&csrf) {
                response
                    .headers_mut()
                    .insert(HeaderName::from_static("x-csrf-token"), value);
            }
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ACCESS_TOKEN_COOKIE, is_public_api_path, latest_access_token};
    use actix_web::test as actix_test;
    use jsonwebtoken::{DecodingKey, EncodingKey};

    use crate::auth::{AccessKeys, AuthKind, Token};

    #[test]
    fn public_api_paths_are_explicit() {
        assert!(is_public_api_path("/api/discord_login"));
        assert!(is_public_api_path("/api/oauth/start"));
        assert!(is_public_api_path("/api/refresh"));
        assert!(is_public_api_path("/api/logout"));
        assert!(!is_public_api_path("/api/users/current"));
        assert!(!is_public_api_path("/api/refresh/extra"));
    }

    #[test]
    fn newest_valid_access_cookie_wins_over_invalid_duplicate()
    -> Result<(), Box<dyn std::error::Error>> {
        let keys = AccessKeys {
            access_encode: EncodingKey::from_secret(b"test_secret"),
            refresh_encode: EncodingKey::from_secret(b"test_secret"),
            access_decode: DecodingKey::from_secret(b"test_secret"),
            refresh_decode: DecodingKey::from_secret(b"test_secret"),
        };
        let valid = Token::<super::Access>::encode(
            42,
            AuthKind::Discord,
            "csrf-current".into(),
            &keys.access_encode,
        )?;
        let req = actix_test::TestRequest::default()
            .insert_header((
                actix_web::http::header::COOKIE,
                format!(
                    "access_token=production-token; {ACCESS_TOKEN_COOKIE}=stale-token; \
                     {ACCESS_TOKEN_COOKIE}={valid}"
                ),
            ))
            .to_srv_request();

        let decoded = latest_access_token(&req, &keys).ok_or("valid duplicate token missing")?;
        assert_eq!(decoded.user_id, 42);
        assert_eq!(decoded.csrf, "csrf-current");
        Ok(())
    }
}

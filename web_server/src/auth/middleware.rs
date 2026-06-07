use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{http::Method, web, Error, HttpMessage, HttpResponse};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use serde_json::json;
use tracing::warn;

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

        let access_cookie = match req.cookie("access_token") {
            Some(c) => c,
            None => {
                warn_unauthorized_middleware_access(req.path(), "missing access_token cookie");
                let (request, _pl) = req.into_parts();
                let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
            }
        };

        let decoded_access = match Token::<Access>::decode(access_cookie.value(), keys) {
            Ok(token) => token,
            Err(_) => {
                warn_unauthorized_middleware_access(req.path(), "invalid or expired token");
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

        req.extensions_mut().insert(decoded_access);
        let res = self.service.call(req);
        Box::pin(async move { res.await.map(ServiceResponse::map_into_left_body) })
    }
}

#[cfg(test)]
mod tests {
    use super::is_public_api_path;

    #[test]
    fn public_api_paths_are_explicit() {
        assert!(is_public_api_path("/api/discord_login"));
        assert!(is_public_api_path("/api/oauth/start"));
        assert!(is_public_api_path("/api/refresh"));
        assert!(is_public_api_path("/api/logout"));
        assert!(!is_public_api_path("/api/users/current"));
        assert!(!is_public_api_path("/api/refresh/extra"));
    }
}

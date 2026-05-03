use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{http::Method, web, Error, HttpMessage, HttpResponse};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use serde_json::json;
use tracing::warn;

use super::jwt::{Access, AccessKeys, Token};

pub struct AuthMiddleware;

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
        let path = req.path();
        let is_dev_login = cfg!(feature = "dev-login") && path == "/api/dev_login";
        if path == "/api/discord_login"
            || is_dev_login
            || path == "/api/refresh"
            || path == "/api/logout"
        {
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
                warn!(
                    "Unauthorized access attempt to middleware {}: missing access_token cookie",
                    req.path()
                );
                let (request, _pl) = req.into_parts();
                let response = HttpResponse::Unauthorized().finish().map_into_right_body();
                return Box::pin(async { Ok(ServiceResponse::new(request, response)) });
            }
        };

        let decoded_access = match Token::<Access>::decode(access_cookie.value(), keys) {
            Ok(token) => token,
            Err(_) => {
                warn!(
                    "Unauthorized access attempt to middleware {}: invalid or expired token",
                    req.path()
                );
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

//! Applied outside CORS so ordinary, error, media, OAuth, and preflight
//! responses all receive the same baseline security policy.

use std::future::{Ready, ready};

use actix_web::{
    Error,
    body::MessageBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{HeaderName, HeaderValue},
};
use futures_util::future::LocalBoxFuture;

pub const DEFAULT_CSP: &str =
    "default-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";
// Scalar loads its viewer from jsDelivr. Keep that exception on the docs page
// only; JSON, media, and OAuth endpoints retain the much tighter policy.
const SCALAR_CSP: &str = "default-src 'none'; base-uri 'none'; frame-ancestors 'none'; script-src https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline'; connect-src 'self' https:; img-src https: data:";
const HSTS: &str = "max-age=63072000; includeSubDomains";
const PERMISSIONS: &str = "camera=(), microphone=(), geolocation=()";

pub struct SecurityHeaders;

impl<S, B> Transform<S, ServiceRequest> for SecurityHeaders
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityHeadersMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SecurityHeadersMiddleware { service }))
    }
}

pub struct SecurityHeadersMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for SecurityHeadersMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let fut = self.service.call(req);
        Box::pin(async move {
            let mut response = fut.await?;
            let docs_page = response.request().path() == "/scalar";
            let headers = response.headers_mut();
            if !headers.contains_key("content-security-policy") {
                headers.insert(
                    HeaderName::from_static("content-security-policy"),
                    HeaderValue::from_static(if docs_page { SCALAR_CSP } else { DEFAULT_CSP }),
                );
            }
            headers.insert(
                HeaderName::from_static("strict-transport-security"),
                HeaderValue::from_static(HSTS),
            );
            headers.insert(
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            );
            headers.insert(
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(
                HeaderName::from_static("referrer-policy"),
                HeaderValue::from_static("no-referrer"),
            );
            headers.insert(
                HeaderName::from_static("permissions-policy"),
                HeaderValue::from_static(PERMISSIONS),
            );
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpResponse, http::StatusCode, test, web};

    #[actix_web::test]
    async fn policy_covers_normal_error_media_callback_and_preflight_responses() {
        let app = test::init_service(
            App::new()
                .route(
                    "/api/normal",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                )
                .route(
                    "/api/error",
                    web::get().to(|| async { HttpResponse::BadRequest().finish() }),
                )
                .route(
                    "/api/media",
                    web::get()
                        .to(|| async { HttpResponse::Ok().content_type("audio/ogg").finish() }),
                )
                .route(
                    "/api/oauth/callback",
                    web::get().to(|| async {
                        HttpResponse::Ok()
                            .insert_header((
                                "content-security-policy",
                                "default-src 'none'; script-src 'nonce-test'",
                            ))
                            .finish()
                    }),
                )
                .route(
                    "/api/preflight",
                    web::method(actix_web::http::Method::OPTIONS)
                        .to(|| async { HttpResponse::NoContent().finish() }),
                )
                .route(
                    "/scalar",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                )
                .wrap(SecurityHeaders),
        )
        .await;
        for (path, expected) in [
            ("/api/normal", StatusCode::OK),
            ("/api/error", StatusCode::BAD_REQUEST),
            ("/api/media", StatusCode::OK),
            ("/api/oauth/callback", StatusCode::OK),
            ("/api/missing", StatusCode::NOT_FOUND),
            ("/scalar", StatusCode::OK),
        ] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(response.status(), expected);
            for key in [
                "content-security-policy",
                "strict-transport-security",
                "x-frame-options",
                "x-content-type-options",
                "referrer-policy",
                "permissions-policy",
            ] {
                assert!(
                    response.headers().contains_key(key),
                    "missing {key} on {path}"
                );
            }
            if path == "/api/oauth/callback" {
                assert!(
                    response
                        .headers()
                        .get("content-security-policy")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .contains("nonce-test")
                );
            } else if path == "/scalar" {
                assert_eq!(
                    response.headers().get("content-security-policy").unwrap(),
                    SCALAR_CSP
                );
            } else {
                assert_eq!(
                    response.headers().get("content-security-policy").unwrap(),
                    DEFAULT_CSP
                );
            }
            assert_eq!(
                response.headers().get("strict-transport-security").unwrap(),
                HSTS
            );
            assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
            assert_eq!(
                response.headers().get("referrer-policy").unwrap(),
                "no-referrer"
            );
            assert_eq!(
                response.headers().get("permissions-policy").unwrap(),
                PERMISSIONS
            );
        }
        let response = test::call_service(
            &app,
            test::TestRequest::default()
                .method(actix_web::http::Method::OPTIONS)
                .uri("/api/preflight")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(response.headers().contains_key("content-security-policy"));
    }
}

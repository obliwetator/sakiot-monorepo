use std::future::{Ready, ready};
use std::sync::OnceLock;
use std::time::Instant;

use actix_web::Error;
use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::header::{HeaderName, HeaderValue};
use futures_util::future::LocalBoxFuture;
use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;

use crate::server_timing::Collector;

static LATENCY_HIST: OnceLock<Histogram<f64>> = OnceLock::new();

fn latency_histogram() -> &'static Histogram<f64> {
    LATENCY_HIST.get_or_init(|| {
        opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
            .f64_histogram("http_server_request_duration")
            .with_description("HTTP server request duration in milliseconds")
            .with_unit("ms")
            .build()
    })
}

/// Records each request's latency and its [`server_timing`](crate::server_timing)
/// segments. With `server_timing_header`, the segments are also returned as a
/// `Server-Timing` header; it exposes how long the server spends on each
/// request, so it stays off in production.
pub struct HttpMetrics {
    pub server_timing_header: bool,
}

impl<S, B> Transform<S, ServiceRequest> for HttpMetrics
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = HttpMetricsMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(HttpMetricsMiddleware {
            service,
            server_timing_header: self.server_timing_header,
        }))
    }
}

pub struct HttpMetricsMiddleware<S> {
    service: S,
    server_timing_header: bool,
}

impl<S, B> Service<ServiceRequest> for HttpMetricsMiddleware<S>
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
        let method = req.method().as_str().to_owned();
        let start = Instant::now();
        let collector = Collector::default();
        let fut = collector.sync_scope(|| self.service.call(req));
        let server_timing_header = self.server_timing_header;

        Box::pin(async move {
            let mut res = collector.scope(fut).await;
            let elapsed = start.elapsed();
            let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

            let (route, status) = match &res {
                Ok(resp) => {
                    let route = resp
                        .request()
                        .match_pattern()
                        .unwrap_or_else(|| "unmatched".to_string());
                    (route, resp.status().as_u16())
                }
                Err(err) => (
                    "unmatched".to_string(),
                    err.as_response_error().status_code().as_u16(),
                ),
            };

            let segments = collector.take();
            segments.export(&route);
            if server_timing_header
                && let Ok(resp) = &mut res
                && let Ok(value) = HeaderValue::from_str(&segments.header_value(elapsed))
            {
                resp.headers_mut()
                    .insert(HeaderName::from_static("server-timing"), value);
            }

            latency_histogram().record(
                elapsed_ms,
                &[
                    KeyValue::new("method", method),
                    KeyValue::new("route", route),
                    KeyValue::new("status", status.to_string()),
                ],
            );

            res
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpResponse, test, web};

    async fn timed_handler() -> HttpResponse {
        crate::server_timing::measure("db", async {}).await;
        HttpResponse::Ok().finish()
    }

    async fn server_timing(header_enabled: bool) -> Option<String> {
        let app = test::init_service(
            App::new()
                .route("/timed", web::get().to(timed_handler))
                .wrap(HttpMetrics {
                    server_timing_header: header_enabled,
                }),
        )
        .await;
        let response =
            test::call_service(&app, test::TestRequest::get().uri("/timed").to_request()).await;
        response
            .headers()
            .get("server-timing")
            .map(|value| value.to_str().unwrap_or_default().to_owned())
    }

    #[actix_web::test]
    async fn the_breakdown_is_returned_only_when_enabled() {
        let header = server_timing(true).await.unwrap_or_default();
        assert!(header.starts_with("db;dur="), "{header}");
        assert!(header.contains(", total;dur="), "{header}");
        assert_eq!(server_timing(false).await, None);
    }
}

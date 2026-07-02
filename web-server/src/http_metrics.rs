use std::future::{Ready, ready};
use std::sync::OnceLock;
use std::time::Instant;

use actix_web::Error;
use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use futures_util::future::LocalBoxFuture;
use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;

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

fn is_websocket_path(path: &str) -> bool {
    path == "/ws/" || path == "/api/dashboard/stream"
}

pub struct HttpMetrics;

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
        ready(Ok(HttpMetricsMiddleware { service }))
    }
}

pub struct HttpMetricsMiddleware<S> {
    service: S,
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
        if is_websocket_path(req.path()) {
            let fut = self.service.call(req);
            return Box::pin(fut);
        }

        let method = req.method().as_str().to_owned();
        let start = Instant::now();
        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

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

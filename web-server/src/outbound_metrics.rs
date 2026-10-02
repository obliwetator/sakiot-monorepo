//! Calls web_server makes to other services:
//! `outbound_request_duration_seconds{target, operation, outcome}`, one sample
//! per call. `target` is `discord` or `fbi_agent`; `outcome` is `ok` or a
//! bounded failure name (an error kind, or a gRPC status code).

use std::future::Future;
use std::sync::OnceLock;
use std::time::Instant;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;

use crate::errors::AppError;

fn histogram() -> &'static Histogram<f64> {
    static HISTOGRAM: OnceLock<Histogram<f64>> = OnceLock::new();
    HISTOGRAM.get_or_init(|| {
        opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
            .f64_histogram("outbound_request_duration_seconds")
            .with_description("Duration of calls to Discord and the FBI agent")
            .with_unit("s")
            .with_boundaries(vec![
                0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
            ])
            .build()
    })
}

/// Records one call that took `seconds`.
pub fn record(target: &'static str, operation: &'static str, outcome: &'static str, seconds: f64) {
    histogram().record(
        seconds,
        &[
            KeyValue::new("target", target),
            KeyValue::new("operation", operation),
            KeyValue::new("outcome", outcome),
        ],
    );
}

/// Runs a Discord call and records it; failures are labelled by error kind.
pub async fn discord<T, F>(operation: &'static str, call: F) -> Result<T, AppError>
where
    F: Future<Output = Result<T, AppError>>,
{
    let started = Instant::now();
    let result = call.await;
    let outcome = match &result {
        Ok(_) => "ok",
        Err(error) => error.kind().as_str(),
    };
    record(
        "discord",
        operation,
        outcome,
        started.elapsed().as_secs_f64(),
    );
    result
}

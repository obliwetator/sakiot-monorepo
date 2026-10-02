//! Background job outcomes: `media_job_duration_seconds{kind, outcome}` gets
//! one sample per attempt, so the histogram's count gives attempts and
//! failures by kind and its buckets give how long each kind takes.
//!
//! `outcome` is `ready`, `lease_lost` (another worker took the job over), or
//! the attempt's error kind, such as `execution_timed_out`.

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;

use crate::errors::AppError;

fn histogram() -> &'static Histogram<f64> {
    static HISTOGRAM: OnceLock<Histogram<f64>> = OnceLock::new();
    HISTOGRAM.get_or_init(|| {
        opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
            .f64_histogram("media_job_duration_seconds")
            .with_description("Duration of one background job attempt, by kind and outcome")
            .with_unit("s")
            .with_boundaries(vec![
                0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0,
            ])
            .build()
    })
}

/// The `outcome` label for an attempt's result.
pub fn outcome<T>(result: &Result<T, AppError>) -> &'static str {
    match result {
        Ok(_) => "ready",
        Err(AppError::JobLeaseLost) => "lease_lost",
        Err(error) => error.kind().as_str(),
    }
}

/// Records one attempt of a `kind` job.
pub fn record(kind: &'static str, outcome: &'static str, elapsed: Duration) {
    histogram().record(
        elapsed.as_secs_f64(),
        &[
            KeyValue::new("kind", kind),
            KeyValue::new("outcome", outcome),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_name_success_takeover_and_the_error_kind() {
        assert_eq!(outcome(&Ok::<(), AppError>(())), "ready");
        assert_eq!(outcome::<()>(&Err(AppError::JobLeaseLost)), "lease_lost");
        assert_eq!(
            outcome::<()>(&Err(AppError::ExecutionTimedOut)),
            "execution_timed_out"
        );
    }
}

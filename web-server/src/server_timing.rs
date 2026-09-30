//! Per-request timing breakdown.
//!
//! [`HttpMetrics`](crate::http_metrics::HttpMetrics) opens a collector for
//! every request, and handlers and shared helpers add named segments to it with
//! [`measure`] or [`record`]. Each segment feeds the
//! `http_server_segment_duration` histogram in every environment. Only when
//! `SAKIOT_SERVER_TIMING_HEADER` is on are the segments also sent back as a
//! `Server-Timing` header, which DevTools shows in a request's Timing tab.
//!
//! A segment is the sum of every call with that name, so a query run once per
//! row shows up as one entry with its call count. Segments of different names
//! may nest (`session` includes `perm`), so they do not add up to the total.

use std::cell::RefCell;
use std::fmt::Write;
use std::future::Future;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;

tokio::task_local! {
    static SEGMENTS: Rc<RefCell<Segments>>;
}

static SEGMENT_HIST: OnceLock<Histogram<f64>> = OnceLock::new();

fn segment_histogram() -> &'static Histogram<f64> {
    SEGMENT_HIST.get_or_init(|| {
        opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
            .f64_histogram("http_server_segment_duration")
            .with_description("Time a request spent in a named segment, in milliseconds")
            .with_unit("ms")
            .build()
    })
}

#[derive(Debug, PartialEq)]
struct Segment {
    name: &'static str,
    total: Duration,
    calls: u32,
}

/// The segments of one request, in the order they first ran.
#[derive(Debug, Default)]
pub(crate) struct Segments(Vec<Segment>);

impl Segments {
    fn add(&mut self, name: &'static str, elapsed: Duration) {
        match self.0.iter_mut().find(|segment| segment.name == name) {
            Some(segment) => {
                segment.total += elapsed;
                segment.calls += 1;
            }
            None => self.0.push(Segment {
                name,
                total: elapsed,
                calls: 1,
            }),
        }
    }

    /// Feeds each segment into the histogram under the matched route.
    pub(crate) fn export(&self, route: &str) {
        for segment in &self.0 {
            segment_histogram().record(
                millis(segment.total),
                &[
                    KeyValue::new("route", route.to_owned()),
                    KeyValue::new("segment", segment.name),
                ],
            );
        }
    }

    /// The `Server-Timing` value: every segment, then the request's `total`.
    pub(crate) fn header_value(&self, total: Duration) -> String {
        let mut value = String::new();
        for segment in &self.0 {
            let _ = write!(value, "{};dur={:.1}", segment.name, millis(segment.total));
            if segment.calls > 1 {
                let _ = write!(value, ";desc=\"{} calls\"", segment.calls);
            }
            value.push_str(", ");
        }
        let _ = write!(value, "total;dur={:.1}", millis(total));
        value
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// A request's collector. Install it around both the synchronous service call
/// and the returned future: middleware such as authentication does its work
/// in `call`, before the future is first polled.
#[derive(Clone, Default)]
pub(crate) struct Collector(Rc<RefCell<Segments>>);

impl Collector {
    pub(crate) fn sync_scope<R>(&self, f: impl FnOnce() -> R) -> R {
        SEGMENTS.sync_scope(self.0.clone(), f)
    }

    pub(crate) async fn scope<F: Future>(&self, fut: F) -> F::Output {
        SEGMENTS.scope(self.0.clone(), fut).await
    }

    pub(crate) fn take(&self) -> Segments {
        self.0.take()
    }
}

/// Adds `elapsed` to the current request's `name` segment. Outside a request,
/// such as in a background worker, this does nothing.
pub fn record(name: &'static str, elapsed: Duration) {
    let _ = SEGMENTS.try_with(|segments| segments.borrow_mut().add(name, elapsed));
}

/// Runs `fut` and adds its duration to the current request's `name` segment.
pub async fn measure<F: Future>(name: &'static str, fut: F) -> F::Output {
    let start = Instant::now();
    let output = fut.await;
    record(name, start.elapsed());
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_segments_add_up_and_count_their_calls() {
        let mut segments = Segments::default();
        segments.add("auth", Duration::from_micros(100));
        segments.add("session", Duration::from_millis(4));
        segments.add("session", Duration::from_millis(6));
        assert_eq!(
            segments.header_value(Duration::from_millis(12)),
            "auth;dur=0.1, session;dur=10.0;desc=\"2 calls\", total;dur=12.0"
        );
    }

    #[test]
    fn a_request_without_segments_reports_only_its_total() {
        assert_eq!(
            Segments::default().header_value(Duration::from_micros(1500)),
            "total;dur=1.5"
        );
    }

    #[actix_web::test]
    async fn measurements_outside_a_request_are_dropped() {
        assert_eq!(measure("db", async { 7 }).await, 7);
        record("db", Duration::from_millis(1));
    }

    #[actix_web::test]
    async fn the_collector_sees_synchronous_and_async_work() {
        let collector = Collector::default();
        let fut = collector.sync_scope(|| {
            record("auth", Duration::from_millis(1));
            async { measure("db", async {}).await }
        });
        collector.scope(fut).await;
        let names: Vec<_> = collector.take().0.iter().map(|s| s.name).collect();
        assert_eq!(names, ["auth", "db"]);
    }
}

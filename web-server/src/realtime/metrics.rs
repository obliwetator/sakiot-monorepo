//! Realtime health, reported through the existing OpenTelemetry exporter.
//! None of it affects `/readyz`: a database hiccup degrades realtime to the
//! clients' polling fallback, it does not take the instance out of service.

use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram};

pub(super) struct Metrics {
    /// 1 while LISTEN is active, 0 while the listener reconnects.
    pub listener_connected: Gauge<u64>,
    pub listener_reconnects: Counter<u64>,
    /// Self-NOTIFY round trip: database to this listener.
    pub db_roundtrip_seconds: Histogram<f64>,
    /// From receiving a batch's first notification to queueing its messages.
    pub delivery_seconds: Histogram<f64>,
    pub resyncs: Counter<u64>,
    pub queue_overflows: Counter<u64>,
    pub authorization_recomputes: Counter<u64>,
    /// `pg_notification_queue_usage()`: a full queue fails every notifying
    /// commit, so alert well before 1.
    pub notification_queue_usage: Gauge<f64>,
}

pub(super) fn metrics() -> &'static Metrics {
    static METRICS: OnceLock<Metrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = opentelemetry::global::meter(crate::telemetry::SERVICE_NAME);
        Metrics {
            listener_connected: meter
                .u64_gauge("realtime_listener_connected")
                .with_description("1 while the realtime database listener is active")
                .build(),
            listener_reconnects: meter
                .u64_counter("realtime_listener_reconnects")
                .with_description("Realtime listener rebuilds after a lost connection")
                .build(),
            db_roundtrip_seconds: meter
                .f64_histogram("realtime_db_roundtrip_seconds")
                .with_description("NOTIFY round trip from the database to the realtime listener")
                .build(),
            delivery_seconds: meter
                .f64_histogram("realtime_delivery_seconds")
                .with_description("Time from receiving a notification batch to queueing its events")
                .build(),
            resyncs: meter
                .u64_counter("realtime_resyncs")
                .with_description("resync_required messages sent, by reason")
                .build(),
            queue_overflows: meter
                .u64_counter("realtime_queue_overflows")
                .with_description("Connections whose outbound queue overflowed")
                .build(),
            authorization_recomputes: meter
                .u64_counter("realtime_authorization_recomputes")
                .with_description("Subscription authorizations computed")
                .build(),
            notification_queue_usage: meter
                .f64_gauge("pg_notification_queue_usage")
                .with_description("Fraction of PostgreSQL's NOTIFY queue in use")
                .build(),
        }
    })
}

pub(super) fn resync(reason: super::protocol::ResyncReason, count: u64) {
    let reason = match reason {
        super::protocol::ResyncReason::ListenerReconnected => "listener_reconnected",
        super::protocol::ResyncReason::QueueOverflow => "queue_overflow",
        super::protocol::ResyncReason::Permissions => "permissions",
    };
    metrics()
        .resyncs
        .add(count, &[KeyValue::new("reason", reason)]);
}

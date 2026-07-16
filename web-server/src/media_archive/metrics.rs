use std::{sync::OnceLock, time::Duration};

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Histogram},
};

use super::repository::ArchiveStatus;

struct ArchiveMetrics {
    pending_objects: Gauge<u64>,
    pending_bytes: Gauge<u64>,
    oldest_backlog_seconds: Gauge<u64>,
    upload_failures: Counter<u64>,
    verification_failures: Counter<u64>,
    transferred_bytes: Counter<u64>,
    transfer_duration: Histogram<f64>,
    remote_read_failures: Counter<u64>,
    evictable_cache_bytes: Gauge<u64>,
}

fn instruments() -> &'static ArchiveMetrics {
    static METRICS: OnceLock<ArchiveMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = opentelemetry::global::meter(crate::telemetry::SERVICE_NAME);
        ArchiveMetrics {
            pending_objects: meter
                .u64_gauge("media_archive_pending_objects")
                .with_description("Media objects waiting for archive verification")
                .build(),
            pending_bytes: meter
                .u64_gauge("media_archive_pending_bytes")
                .with_description("Known bytes waiting for archive verification")
                .with_unit("By")
                .build(),
            oldest_backlog_seconds: meter
                .u64_gauge("media_archive_oldest_backlog_age")
                .with_description("Age of oldest pending media object")
                .with_unit("s")
                .build(),
            upload_failures: meter
                .u64_counter("media_archive_upload_failures")
                .with_description("Failed B2 media uploads")
                .build(),
            verification_failures: meter
                .u64_counter("media_archive_verification_failures")
                .with_description("Failed full-object B2 SHA-256 verifications")
                .build(),
            transferred_bytes: meter
                .u64_counter("media_archive_transferred_bytes")
                .with_description("Bytes transferred between local storage and B2")
                .with_unit("By")
                .build(),
            transfer_duration: meter
                .f64_histogram("media_archive_transfer_duration")
                .with_description("B2 media transfer duration")
                .with_unit("s")
                .build(),
            remote_read_failures: meter
                .u64_counter("media_archive_remote_read_failures")
                .with_description("Failed runtime reads from B2")
                .build(),
            evictable_cache_bytes: meter
                .u64_gauge("media_archive_evictable_cache_bytes")
                .with_description("Verified local media bytes eligible for safe eviction")
                .with_unit("By")
                .build(),
        }
    })
}

pub(crate) fn record_status(status: &ArchiveStatus) {
    let metrics = instruments();
    metrics.pending_objects.record(
        nonnegative(status.pending_objects + status.uploading_objects),
        &[],
    );
    metrics
        .pending_bytes
        .record(nonnegative(status.pending_bytes), &[]);
    metrics
        .oldest_backlog_seconds
        .record(nonnegative(status.oldest_backlog_seconds), &[]);
}

pub(crate) fn upload_failure() {
    instruments().upload_failures.add(1, &[]);
}

pub(crate) fn verification_failure() {
    instruments().verification_failures.add(1, &[]);
}

pub(crate) fn transfer_success(direction: &'static str, bytes: u64, duration: Duration) {
    let attributes = [KeyValue::new("direction", direction)];
    instruments().transferred_bytes.add(bytes, &attributes);
    instruments()
        .transfer_duration
        .record(duration.as_secs_f64(), &attributes);
}

pub(crate) fn remote_read_success(bytes: u64, duration: Duration) {
    transfer_success("download", bytes, duration);
}

pub(crate) fn remote_read_failure() {
    instruments().remote_read_failures.add(1, &[]);
}

pub(crate) fn record_evictable_cache_bytes(bytes: u64) {
    instruments().evictable_cache_bytes.record(bytes, &[]);
}

fn nonnegative(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

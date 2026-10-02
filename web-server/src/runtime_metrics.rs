//! Tokio runtime health, per runtime: worker count, live tasks, tasks queued
//! for a worker, blocking-pool threads, and total time workers spent busy.
//! `rate(tokio_runtime_busy_seconds_total) / tokio_runtime_workers` is the
//! share of time the runtime was busy; a runtime pinned near 1 has code that
//! blocks it, delaying everything else scheduled on it.

use opentelemetry::KeyValue;

/// Reports the current thread's tokio runtime as `runtime`.
pub fn observe_current(runtime: String) {
    let metrics = tokio::runtime::Handle::current().metrics();
    let meter = opentelemetry::global::meter(crate::telemetry::SERVICE_NAME);
    let labels = [KeyValue::new("runtime", runtime)];

    let (m, l) = (metrics.clone(), labels.clone());
    meter
        .u64_observable_gauge("tokio_runtime_workers")
        .with_description("Worker threads of the tokio runtime")
        .with_callback(move |o| o.observe(m.num_workers() as u64, &l))
        .build();
    let (m, l) = (metrics.clone(), labels.clone());
    meter
        .u64_observable_gauge("tokio_runtime_alive_tasks")
        .with_description("Tasks alive on the tokio runtime")
        .with_callback(move |o| o.observe(m.num_alive_tasks() as u64, &l))
        .build();
    let (m, l) = (metrics.clone(), labels.clone());
    meter
        .u64_observable_gauge("tokio_runtime_global_queue_depth")
        .with_description("Tasks waiting in the runtime's global queue for a worker")
        .with_callback(move |o| o.observe(m.global_queue_depth() as u64, &l))
        .build();
    // Needs tokio_unstable, which .cargo/config.toml sets for every build.
    let (m, l) = (metrics.clone(), labels.clone());
    meter
        .u64_observable_gauge("tokio_runtime_blocking_threads")
        .with_description("Threads in the runtime's blocking pool")
        .with_callback(move |o| o.observe(m.num_blocking_threads() as u64, &l))
        .build();
    meter
        .f64_observable_counter("tokio_runtime_busy_seconds")
        .with_description("Total time the runtime's workers spent running tasks")
        .with_unit("s")
        .with_callback(move |o| {
            let busy: f64 = (0..metrics.num_workers())
                .map(|worker| metrics.worker_total_busy_duration(worker).as_secs_f64())
                .sum();
            o.observe(busy, &labels);
        })
        .build();
}

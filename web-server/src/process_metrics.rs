//! This process's CPU, memory and file descriptors, read from `/proc/self`
//! when metrics are exported. Host-wide CPU cannot tell the web server apart
//! from the agent, PostgreSQL or the other services on the machine; these can.
//!
//! - `web_server_process_cpu_seconds_total{mode}`: `user` and `system` for
//!   this process, `children_user` and `children_system` for the child
//!   processes it has waited for (ffmpeg, composition workers). A child's
//!   time is counted when it exits, so a long-running live HLS mux shows up
//!   at its end. Decoding audio for waveform peaks runs in this process.
//! - `web_server_process_resident_memory_bytes`, `web_server_process_open_fds`,
//!   `web_server_process_threads`.
//!
//! Elsewhere than Linux nothing is reported.

use opentelemetry::KeyValue;

pub fn observe() {
    let meter = opentelemetry::global::meter(crate::telemetry::SERVICE_NAME);
    meter
        .f64_observable_counter("web_server_process_cpu_seconds")
        .with_description("CPU time of this process and its waited-for children")
        .with_unit("s")
        .with_callback(|o| {
            if let Some(cpu) = sakiot_process::cpu_seconds() {
                for (mode, seconds) in [
                    ("user", cpu.user),
                    ("system", cpu.system),
                    ("children_user", cpu.children_user),
                    ("children_system", cpu.children_system),
                ] {
                    o.observe(seconds, &[KeyValue::new("mode", mode)]);
                }
            }
        })
        .build();
    meter
        .u64_observable_gauge("web_server_process_resident_memory_bytes")
        .with_description("Resident set size of this process")
        .with_unit("By")
        .with_callback(|o| {
            if let Some(bytes) = sakiot_process::status_value("VmRSS:").map(|kib| kib * 1024) {
                o.observe(bytes, &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge("web_server_process_threads")
        .with_description("Threads of this process")
        .with_callback(|o| {
            if let Some(threads) = sakiot_process::status_value("Threads:") {
                o.observe(threads, &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge("web_server_process_open_fds")
        .with_description("Open file descriptors of this process")
        .with_callback(|o| {
            if let Some(fds) = sakiot_process::open_fds() {
                o.observe(fds, &[]);
            }
        })
        .build();
}

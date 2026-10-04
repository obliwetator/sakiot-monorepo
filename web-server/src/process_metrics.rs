//! This process's CPU, memory and file descriptors, read from `/proc/self`
//! when metrics are exported. Host-wide CPU cannot tell the web server apart
//! from the agent, PostgreSQL or the other services on the machine; these can.
//!
//! - `web_server_process_cpu_seconds_total{mode}`: `user` and `system` for
//!   this process, `children_user` and `children_system` for the child
//!   processes it has waited for (ffmpeg, audiowaveform, composition
//!   workers). A child's time is counted when it exits, so a long-running
//!   live HLS mux shows up at its end.
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
            if let Some(cpu) = cpu_seconds() {
                for (mode, seconds) in cpu {
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
            if let Some(bytes) = status_kib("VmRSS:").map(|kib| kib * 1024) {
                o.observe(bytes, &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge("web_server_process_threads")
        .with_description("Threads of this process")
        .with_callback(|o| {
            if let Some(threads) = status_kib("Threads:") {
                o.observe(threads, &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge("web_server_process_open_fds")
        .with_description("Open file descriptors of this process")
        .with_callback(|o| {
            if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
                o.observe(entries.count() as u64, &[]);
            }
        })
        .build();
}

/// `/proc` reports CPU time in USER_HZ ticks, which Linux fixes at 100 on
/// every architecture it exports to user space (`sysconf(_SC_CLK_TCK)`).
const USER_HZ: f64 = 100.0;

/// utime, stime, cutime and cstime from `/proc/self/stat`, in seconds.
fn cpu_seconds() -> Option<[(&'static str, f64); 4]> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    parse_stat(&stat, USER_HZ)
}

fn parse_stat(stat: &str, ticks_per_second: f64) -> Option<[(&'static str, f64); 4]> {
    // The command name (field 2) is parenthesized and may contain spaces;
    // fields after it are space-separated, utime being field 14.
    let rest = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |n: usize| -> Option<f64> { fields.get(n - 3)?.parse::<f64>().ok() };
    Some([
        ("user", field(14)? / ticks_per_second),
        ("system", field(15)? / ticks_per_second),
        ("children_user", field(16)? / ticks_per_second),
        ("children_system", field(17)? / ticks_per_second),
    ])
}

/// A `Name:  value kB` (or plain count) line of `/proc/self/status`.
fn status_kib(name: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with(name))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::parse_stat;

    #[test]
    fn stat_fields_are_read_after_the_command_name() {
        let stat = "4242 (web server) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 30 10 20 0 12 0";
        let cpu = parse_stat(stat, 100.0).map(|fields| fields.map(|(_, value)| value));
        assert_eq!(cpu, Some([2.5, 0.5, 0.3, 0.1]));
    }
}

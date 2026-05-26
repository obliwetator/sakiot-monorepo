use std::sync::Arc;

use super::bot::BotMetrics;

impl BotMetrics {
    /// Starts a background task to monitor process health (memory, FDs, active tasks).
    pub fn start_sysinfo_monitoring(metrics: Arc<Self>) {
        tokio::spawn(async move {
            let mut sys = sysinfo::System::new();
            let pid = sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from_u32(0));

            loop {
                // Memory (RSS) from sysinfo
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    sysinfo::ProcessRefreshKind::nothing().with_memory(),
                );
                if let Some(process) = sys.process(pid) {
                    metrics
                        .process_rss_bytes
                        .store(process.memory(), std::sync::atomic::Ordering::Relaxed);
                }

                // Open file descriptors: count entries in /proc/self/fd on linux
                #[cfg(target_os = "linux")]
                if let Ok(mut dir) = tokio::fs::read_dir("/proc/self/fd").await {
                    let mut count: u32 = 0;
                    while dir.next_entry().await.ok().flatten().is_some() {
                        count += 1;
                    }
                    metrics
                        .process_open_fds
                        .store(count, std::sync::atomic::Ordering::Relaxed);
                }

                // Tokio runtime task count (requires tokio_unstable)
                let task_count = tokio::runtime::Handle::current()
                    .metrics()
                    .num_alive_tasks() as u32;
                metrics
                    .tokio_active_tasks
                    .store(task_count, std::sync::atomic::Ordering::Relaxed);

                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
    }
}

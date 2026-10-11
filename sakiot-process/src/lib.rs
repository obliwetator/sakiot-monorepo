//! This process's resource use, read from `/proc/self` (Linux only; `None`
//! elsewhere). Host-wide figures cannot tell the bot, the web server,
//! PostgreSQL and the rest of the VPS apart; these can.

/// `/proc` reports CPU time in USER_HZ ticks, which Linux fixes at 100 on
/// every architecture it exports to user space (`sysconf(_SC_CLK_TCK)`).
const USER_HZ: f64 = 100.0;

/// CPU time, in seconds, of this process and of the children it has waited
/// for. A child's time counts once it exits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuSeconds {
    pub user: f64,
    pub system: f64,
    pub children_user: f64,
    pub children_system: f64,
}

/// I/O this process has caused, from `/proc/self/io`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoCounters {
    /// Bytes fetched from storage (page cache hits excluded).
    pub read_bytes: u64,
    /// Bytes sent to storage: what it dirtied in the page cache, before
    /// filesystem journaling.
    pub write_bytes: u64,
    /// `read`-family system calls.
    pub read_syscalls: u64,
    /// `write`-family system calls.
    pub write_syscalls: u64,
}

pub fn cpu_seconds() -> Option<CpuSeconds> {
    parse_stat(&std::fs::read_to_string("/proc/self/stat").ok()?, USER_HZ)
}

/// `None` on kernels built without task I/O accounting, such as WSL's.
pub fn io_counters() -> Option<IoCounters> {
    parse_io(&std::fs::read_to_string("/proc/self/io").ok()?)
}

/// The number on a `Name:  value [kB]` line of `/proc/self/status`, such as
/// `VmRSS:` (in KiB) or `Threads:`.
pub fn status_value(name: &str) -> Option<u64> {
    parse_status(&std::fs::read_to_string("/proc/self/status").ok()?, name)
}

pub fn open_fds() -> Option<u64> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count() as u64)
}

fn parse_stat(stat: &str, ticks_per_second: f64) -> Option<CpuSeconds> {
    // The command name (field 2) is parenthesized and may contain spaces;
    // fields after it are space-separated, utime being field 14.
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |n: usize| -> Option<f64> { fields.get(n - 3)?.parse::<f64>().ok() };
    Some(CpuSeconds {
        user: field(14)? / ticks_per_second,
        system: field(15)? / ticks_per_second,
        children_user: field(16)? / ticks_per_second,
        children_system: field(17)? / ticks_per_second,
    })
}

fn parse_io(io: &str) -> Option<IoCounters> {
    let value = |name: &str| -> Option<u64> {
        io.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))?
            .trim()
            .parse()
            .ok()
    };
    Some(IoCounters {
        read_bytes: value("read_bytes")?,
        write_bytes: value("write_bytes")?,
        read_syscalls: value("syscr")?,
        write_syscalls: value("syscw")?,
    })
}

fn parse_status(status: &str, name: &str) -> Option<u64> {
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
    use super::*;

    #[test]
    fn stat_fields_are_read_after_the_command_name() {
        let stat = "4242 (web server) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 30 10 20 0 12 0";
        assert_eq!(
            parse_stat(stat, 100.0),
            Some(CpuSeconds {
                user: 2.5,
                system: 0.5,
                children_user: 0.3,
                children_system: 0.1,
            })
        );
        assert_eq!(parse_stat("4242 (cut", 100.0), None);
    }

    #[test]
    fn io_counters_are_read_by_name() {
        let io = "rchar: 9000\nwchar: 8000\nsyscr: 12\nsyscw: 34\n\
                  read_bytes: 4096\nwrite_bytes: 65536\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_io(io),
            Some(IoCounters {
                read_bytes: 4096,
                write_bytes: 65536,
                read_syscalls: 12,
                write_syscalls: 34,
            })
        );
        assert_eq!(parse_io("rchar: 1\n"), None);
    }

    #[test]
    fn status_values_drop_their_unit() {
        let status = "Name:\tfbi_agent\nVmRSS:\t  51200 kB\nThreads:\t17\n";
        assert_eq!(parse_status(status, "VmRSS:"), Some(51_200));
        assert_eq!(parse_status(status, "Threads:"), Some(17));
        assert_eq!(parse_status(status, "VmSwap:"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reads_its_own_figures() {
        assert!(cpu_seconds().is_some());
        assert!(status_value("Threads:").is_some_and(|threads| threads >= 1));
        assert!(open_fds().is_some_and(|fds| fds >= 3));
    }
}

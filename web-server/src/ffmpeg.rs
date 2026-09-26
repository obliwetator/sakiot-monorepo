//! Shared FFmpeg/FFprobe process boundaries.
//!
//! Every call site used to restate the same "is the tool installed?" mapping,
//! which is the difference between an actionable 503 and an opaque 500, plus
//! the same ffprobe argument list and duration parser. The completion runners
//! below also own the spawn/stdio/kill-on-drop plumbing, the `-progress` line
//! filtering, and the capped stderr collection that used to be copy-pasted at
//! each call site.

use std::future::Future;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::errors::AppError;

/// How much FFmpeg stderr is retained for a failure message. Lines are added
/// whole while there is room, so a pathological run cannot balloon memory.
const MAX_ERROR_BYTES: usize = 4096;

/// Map a tool spawn failure, separating a missing FFmpeg install from any
/// other I/O error.
pub(crate) fn tool_error(tool: &str, error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AppError::ServiceUnavailable(format!(
            "{tool} executable is unavailable; install FFmpeg on the web server"
        ))
    } else {
        AppError::IoError(error)
    }
}

/// Run ffprobe for a container duration. Only process-level failures are
/// errors here; a non-zero exit is returned to the caller as a failed status.
pub(crate) async fn run_ffprobe(path: &Path) -> Result<std::process::Output, AppError> {
    tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .map_err(|error| tool_error("ffprobe", error))
}

/// Parse the `format=duration` value emitted by ffprobe. `None` covers the
/// non-numeric (`N/A`), non-finite, and non-positive outputs that all mean "no
/// usable audio in this file".
pub(crate) fn parse_probe_duration(stdout: &[u8]) -> Option<f64> {
    String::from_utf8_lossy(stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
}

/// Run an FFmpeg command to completion. Stdin and stdout are closed, stderr is
/// read while the process runs (so a chatty run cannot deadlock on a full
/// pipe), and a non-zero exit becomes [`AppError::FfmpegError`] carrying the
/// capped stderr as the message.
pub(crate) async fn run_ffmpeg(command: Command) -> Result<(), AppError> {
    run_ffmpeg_with_progress(command, |_| std::future::ready(())).await
}

/// Like [`run_ffmpeg`], additionally reporting each `out_time_us` value from
/// `-progress pipe:2` to `on_progress` while FFmpeg runs. Progress reports
/// never enter the collected diagnostics.
///
/// The callback is `FnMut(u64) -> Future` rather than an async closure so the
/// returned futures have one concrete `Send` type; async closures are
/// higher-ranked over their `&mut self` borrow, which trips "implementation of
/// `Send` is not general enough" once the run sits inside a spawned job.
pub(crate) async fn run_ffmpeg_with_progress<F, Fut>(
    command: Command,
    mut on_progress: F,
) -> Result<(), AppError>
where
    F: FnMut(u64) -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    let mut command = command;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| tool_error("ffmpeg", error))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::FfmpegError("FFmpeg stderr pipe unavailable".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut collector = StderrCollector::default();
    while let Some(line) = lines.next_line().await.map_err(AppError::IoError)? {
        if is_progress_line(&line) {
            if let Some(elapsed_us) = parse_out_time_us(&line) {
                on_progress(elapsed_us).await;
            }
        } else {
            collector.push_line(&line);
        }
    }
    let status = child.wait().await.map_err(AppError::IoError)?;
    if !status.success() {
        return Err(AppError::FfmpegError(collector.failure_message(&status)));
    }
    Ok(())
}

/// A `-progress` report line: `key=value` with one of the keys FFmpeg emits
/// per report. These lines are filtered out of the collected diagnostics.
fn is_progress_line(line: &str) -> bool {
    matches!(
        line.split_once('=').map(|(key, _)| key),
        Some(
            "bitrate"
                | "drop_frames"
                | "dup_frames"
                | "fps"
                | "frame"
                | "out_time"
                | "out_time_ms"
                | "out_time_us"
                | "progress"
                | "speed"
                | "stream_0_0_q"
                | "total_size"
        )
    )
}

/// The `out_time_us` value of a progress report, if the line is one and the
/// value parses.
fn parse_out_time_us(line: &str) -> Option<u64> {
    line.strip_prefix("out_time_us=")?.parse().ok()
}

/// Collects diagnostic stderr for a failure message, capped at
/// [`MAX_ERROR_BYTES`].
#[derive(Default)]
struct StderrCollector {
    output: Vec<u8>,
}

impl StderrCollector {
    /// Append one line plus its newline while there is room. A line longer
    /// than the remaining budget is truncated rather than dropped.
    fn push_line(&mut self, line: &str) {
        if self.output.len() < MAX_ERROR_BYTES {
            let remaining = MAX_ERROR_BYTES - self.output.len();
            self.output
                .extend_from_slice(&line.as_bytes()[..line.len().min(remaining)]);
            self.output.push(b'\n');
        }
    }

    /// The failure message: the collected stderr, trimmed; or the exit status
    /// when FFmpeg failed without saying anything.
    fn failure_message(&self, status: &ExitStatus) -> String {
        let stderr = String::from_utf8_lossy(&self.output);
        let message = stderr.trim();
        if message.is_empty() {
            format!("ffmpeg exited with {status}")
        } else {
            message.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    #[test]
    fn progress_lines_are_classified() {
        for line in [
            "out_time_us=123",
            "out_time_ms=123",
            "out_time=00:00:00.000123",
            "progress=end",
            "speed=1.5x",
            "total_size=12345",
            "bitrate=96.0kbits/s",
        ] {
            assert!(is_progress_line(line), "{line} should be a progress line");
        }
        assert!(!is_progress_line("Error opening input file"));
        assert!(!is_progress_line("no-key-line"));
    }

    #[test]
    fn out_time_values_are_parsed() {
        assert_eq!(parse_out_time_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_out_time_us("out_time_us=N/A"), None);
        assert_eq!(parse_out_time_us("speed=1x"), None);
    }

    #[test]
    fn stderr_lines_build_the_failure_message() {
        let mut collector = StderrCollector::default();
        collector.push_line("invalid input");
        collector.push_line("second line");
        assert_eq!(
            collector.failure_message(&exit_status(1)),
            "invalid input\nsecond line"
        );
    }

    #[test]
    fn silent_failure_reports_the_exit_status() {
        let collector = StderrCollector::default();
        let message = collector.failure_message(&exit_status(1));
        assert!(message.starts_with("ffmpeg exited with"), "{message}");
    }

    #[test]
    fn collection_is_capped() {
        let mut collector = StderrCollector::default();
        // Nine bytes per line (eight plus the newline): more than the budget.
        for _ in 0..(MAX_ERROR_BYTES / 8 + 8) {
            collector.push_line("12345678");
        }
        assert!(collector.output.len() <= MAX_ERROR_BYTES + 1);
        assert!(String::from_utf8_lossy(&collector.output).ends_with('\n'));
    }
}

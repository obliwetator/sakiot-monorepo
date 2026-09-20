//! Shared FFmpeg/FFprobe process boundaries.
//!
//! Every call site used to restate the same "is the tool installed?" mapping,
//! which is the difference between an actionable 503 and an opaque 500, plus
//! the same ffprobe argument list and duration parser.

use std::path::Path;

use crate::errors::AppError;

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

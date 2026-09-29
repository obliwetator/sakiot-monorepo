pub mod admin_api;
pub mod clock;
pub mod components;
pub mod config;
pub mod deploy;
pub mod fsx;
pub mod git;
pub mod lock;
pub mod promotion;
pub mod release;
pub mod runner;
#[expect(
    clippy::print_stdout,
    reason = "the status subcommand's stdout report is its product"
)]
pub mod status;
pub mod systemctl;
pub mod validate;
pub mod web_api;

use std::fmt::Display;
use std::io::Write;

/// Reports deploy progress on stdout.
///
/// Write errors are ignored. When the CI job's SSH client goes away, stdout is
/// a closed pipe; `println!` would panic there (Rust ignores SIGPIPE) and cut
/// short whatever step was logging, including failure handling.
pub fn log(message: impl Display) {
    let _ = writeln!(std::io::stdout(), "[deploy] {message}");
}

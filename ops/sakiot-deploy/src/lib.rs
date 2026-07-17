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

/// Mirrors `log()` from ops/lib/common.sh.
#[expect(
    clippy::print_stdout,
    reason = "deploy progress is reported on stdout, mirroring the bash engine"
)]
pub fn log(message: impl Display) {
    println!("[deploy] {message}");
}

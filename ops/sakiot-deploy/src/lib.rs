pub mod admin_api;
pub mod clock;
pub mod components;
pub mod config;
pub mod deploy;
pub mod fsx;
pub mod git;
pub mod lock;
pub mod release;
pub mod runner;
pub mod systemctl;
pub mod validate;
pub mod web_api;

use std::fmt::Display;

/// Mirrors `log()` from ops/lib/common.sh.
pub fn log(message: impl Display) {
    println!("[deploy] {message}");
}

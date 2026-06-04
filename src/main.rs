mod app;
mod app_state;
pub mod metrics;
pub mod reaper;
pub use app_state::{Custom, HasBossMusic};
pub use metrics::*;

pub mod commands;
pub mod config;
pub mod cooldown;
mod database;
pub mod deployment;
pub mod event_handler;
pub mod events;
pub mod grpc;
pub mod heartbeat;
pub mod runtime;
mod shutdown;
pub mod telemetry;

#[cfg(test)]
mod tests;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    app::run().await
}

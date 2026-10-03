//! Realtime dashboard updates: committed database writes notify
//! (`sakiot_realtime`), one listener per process routes them to authorized
//! WebSocket subscribers, and clients refetch through the HTTP endpoints.
//! See `docs/architecture.md` ("Realtime updates").

pub mod events;
mod hub;
mod listener;
mod metrics;
pub mod protocol;
pub mod socket;

use std::sync::Arc;

pub use hub::{Connection, Hub};
pub use listener::spawn as spawn_listener;
pub use socket::{origin_allowed, realtime_socket};

/// Reports the number of open realtime connections on every export.
pub fn observe(hub: &Arc<Hub>) {
    let hub = Arc::downgrade(hub);
    opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
        .u64_observable_gauge("realtime_connections")
        .with_description("Open realtime WebSocket connections")
        .with_callback(move |observer| {
            if let Some(hub) = hub.upgrade() {
                observer.observe(hub.connection_count() as u64, &[]);
            }
        })
        .build();
}

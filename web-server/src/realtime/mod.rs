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

use opentelemetry::KeyValue;

pub use hub::{Connection, ConnectionStates, Hub};
pub use listener::spawn as spawn_listener;
pub use socket::{origin_allowed, realtime_socket};

/// Reports the open realtime connections on every export, by subscription
/// state (`subscribed`, `refused`, `unscoped`).
pub fn observe(hub: &Arc<Hub>) {
    let hub = Arc::downgrade(hub);
    opentelemetry::global::meter(crate::telemetry::SERVICE_NAME)
        .u64_observable_gauge("realtime_connections")
        .with_description("Open realtime WebSocket connections, by subscription state")
        .with_callback(move |observer| {
            if let Some(hub) = hub.upgrade() {
                let states = hub.connection_states();
                for (state, count) in [
                    ("subscribed", states.subscribed),
                    ("refused", states.refused),
                    ("unscoped", states.unscoped),
                ] {
                    observer.observe(count as u64, &[KeyValue::new("state", state)]);
                }
            }
        })
        .build();
}

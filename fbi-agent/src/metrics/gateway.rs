//! Discord gateway health: `discord_gateway_latency_seconds{shard}`, the time
//! between each shard's last heartbeat and Discord's acknowledgement. A rising
//! value means Discord or the network path to it is slow; a shard without a
//! value has not completed a heartbeat yet.

use std::sync::Arc;

use opentelemetry::KeyValue;
use serenity::gateway::ShardManager;

pub fn observe_gateway_latency(shard_manager: Arc<ShardManager>) {
    opentelemetry::global::meter(crate::config::SERVICE_NAME)
        .f64_observable_gauge("discord_gateway_latency_seconds")
        .with_description("Last heartbeat round trip to the Discord gateway, per shard")
        .with_unit("s")
        .with_callback(move |observer| {
            // The runners map is behind an async mutex; skip this export
            // rather than block the metrics thread while a shard holds it.
            let Ok(runners) = shard_manager.runners.try_lock() else {
                return;
            };
            for (id, runner) in runners.iter() {
                if let Some(latency) = runner.latency {
                    observer.observe(
                        latency.as_secs_f64(),
                        &[KeyValue::new("shard", i64::from(id.0))],
                    );
                }
            }
        })
        .build();
}

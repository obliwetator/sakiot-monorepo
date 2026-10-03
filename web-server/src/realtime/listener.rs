//! One PostgreSQL listener per web-server process.
//!
//! Three tasks on the main runtime:
//! * the listener owns the `PgListener` and only ever awaits `try_recv`, so a
//!   notification is never lost to a cancelled read; it forwards everything
//!   into a bounded channel;
//! * the batcher coalesces what arrives within [`BATCH_WINDOW`] and hands it to
//!   the hub;
//! * the probe measures the database-to-listener round trip and samples
//!   `pg_notification_queue_usage()`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::{Pool, Postgres};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::events::{self, CHANNEL, Event};
use super::hub::Hub;
use super::metrics::metrics;
use super::protocol::ResyncReason;

/// Notifications arriving this close together are routed as one batch, so a
/// burst (a recorder finalizing several fragments) becomes one message per
/// resource per connection.
const BATCH_WINDOW: Duration = Duration::from_millis(100);
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

enum Signal {
    Notification(String),
    /// Continuity was lost: notifications may have been missed. LISTEN is
    /// already active again when this is sent.
    Gap,
}

pub fn spawn(pool: &Pool<Postgres>, hub: Arc<Hub>) -> Vec<JoinHandle<()>> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::channel(4096);
    // A dedicated single-connection pool: the listener holds its connection
    // for as long as it lives, and reconnecting must not queue behind HTTP
    // requests when the main pool is busy.
    let listener_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with((*pool.connect_options()).clone());
    vec![
        tokio::spawn(listen(listener_pool, sender)),
        tokio::spawn(batch(receiver, hub, nonce.clone())),
        tokio::spawn(probe(pool.clone(), nonce)),
    ]
}

async fn connect(pool: &Pool<Postgres>) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(CHANNEL).await?;
    Ok(listener)
}

async fn listen(pool: Pool<Postgres>, sender: mpsc::Sender<Signal>) {
    let mut backoff = Duration::from_secs(1);
    let mut connected_before = false;
    loop {
        let mut listener = match connect(&pool).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%error, ?backoff, "realtime listener cannot connect");
                metrics().listener_connected.record(0, &[]);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        backoff = Duration::from_secs(1);
        metrics().listener_connected.record(1, &[]);
        if connected_before {
            // Rebuilt after an error: whatever happened meanwhile is lost.
            metrics().listener_reconnects.add(1, &[]);
            if sender.send(Signal::Gap).await.is_err() {
                return;
            }
        }
        connected_before = true;

        loop {
            // Eager reconnect (the default) re-establishes LISTEN before
            // `Ok(None)` returns, so the gap is closed by the time it is
            // reported. Turning it off can lose notifications silently.
            match listener.try_recv().await {
                Ok(Some(notification)) => {
                    let payload = notification.payload().to_owned();
                    if sender.send(Signal::Notification(payload)).await.is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    metrics().listener_reconnects.add(1, &[]);
                    if sender.send(Signal::Gap).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "realtime listener lost its connection; rebuilding");
                    metrics().listener_connected.record(0, &[]);
                    break;
                }
            }
        }
    }
}

async fn batch(mut receiver: mpsc::Receiver<Signal>, hub: Arc<Hub>, nonce: String) {
    while let Some(first) = receiver.recv().await {
        let started = std::time::Instant::now();
        let mut batch = Batch::default();
        batch.absorb(first, &nonce);
        let window = tokio::time::sleep(BATCH_WINDOW);
        tokio::pin!(window);
        loop {
            tokio::select! {
                biased;
                signal = receiver.recv() => match signal {
                    Some(signal) => batch.absorb(signal, &nonce),
                    None => break,
                },
                () = &mut window => break,
            }
        }

        if batch.gap {
            // A full resync covers every event in the batch.
            hub.resync_all(ResyncReason::ListenerReconnected).await;
        } else if !batch.events.is_empty() {
            hub.dispatch(batch.events).await;
        }
        metrics()
            .delivery_seconds
            .record(started.elapsed().as_secs_f64(), &[]);
    }
}

#[derive(Default)]
struct Batch {
    events: Vec<Event>,
    seen: HashSet<Event>,
    gap: bool,
}

impl Batch {
    fn absorb(&mut self, signal: Signal, nonce: &str) {
        match signal {
            Signal::Gap => self.gap = true,
            Signal::Notification(payload) => match events::parse(&payload) {
                Some(Event::Probe {
                    nonce: probe_nonce,
                    sent_at_ms,
                }) => {
                    if probe_nonce == nonce {
                        let elapsed_ms = chrono::Utc::now().timestamp_millis() - sent_at_ms;
                        metrics()
                            .db_roundtrip_seconds
                            .record(elapsed_ms.max(0) as f64 / 1000.0, &[]);
                    }
                }
                Some(event) => {
                    if self.seen.insert(event.clone()) {
                        self.events.push(event);
                    }
                }
                None => tracing::debug!(%payload, "ignoring unrecognized realtime payload"),
            },
        }
    }
}

async fn probe(pool: Pool<Postgres>, nonce: String) {
    let mut ticker = tokio::time::interval(PROBE_INTERVAL);
    loop {
        ticker.tick().await;
        let payload = serde_json::json!({
            "v": 1,
            "k": "probe",
            "n": nonce,
            "t": chrono::Utc::now().timestamp_millis(),
        })
        .to_string();
        if let Err(error) = sqlx::query!("SELECT pg_notify($1, $2)", CHANNEL, payload)
            .execute(&pool)
            .await
        {
            tracing::debug!(%error, "realtime probe failed");
        }
        match sqlx::query_scalar!(r#"SELECT pg_notification_queue_usage() AS "usage!""#)
            .fetch_one(&pool)
            .await
        {
            Ok(usage) => metrics().notification_queue_usage.record(usage, &[]),
            Err(error) => tracing::debug!(%error, "notification queue usage unavailable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_coalesces_duplicates_and_keeps_order() {
        let mut batch = Batch::default();
        for payload in [
            r#"{"v":1,"k":"session","g":"1","s":"2","c":"3"}"#,
            r#"{"v":1,"k":"recording","g":"1"}"#,
            r#"{"v":1,"k":"session","g":"1","s":"2","c":"4"}"#,
            r#"{"v":1,"k":"probe","n":"mine","t":0}"#,
            "garbage",
        ] {
            batch.absorb(Signal::Notification(payload.into()), "mine");
        }
        assert_eq!(
            batch.events,
            vec![
                Event::Session {
                    guild_id: 1,
                    session_id: 2
                },
                Event::Recording { guild_id: 1 },
            ]
        );
        assert!(!batch.gap);
        batch.absorb(Signal::Gap, "mine");
        assert!(batch.gap);
    }
}

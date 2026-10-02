//! Database connection pool metrics.
//!
//! `db_pool_connections{state}` reports open and idle connections against the
//! pool's limit: no idle connections at the limit means requests are waiting
//! for one. `db_pool_connections_opened` counts new connections; a steady
//! climb means idle connections keep closing and reopening, and each reopen
//! costs the request that triggers it a TLS and authentication round trip.

use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};

fn opened() -> &'static Counter<u64> {
    static OPENED: OnceLock<Counter<u64>> = OnceLock::new();
    OPENED.get_or_init(|| {
        opentelemetry::global::meter(crate::config::SERVICE_NAME)
            .u64_counter("db_pool_connections_opened")
            .with_description("New database connections opened by the pool")
            .build()
    })
}

/// Pool options that count every connection the pool opens.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new().after_connect(|_connection, _metadata| {
        Box::pin(async {
            opened().add(1, &[]);
            Ok(())
        })
    })
}

/// Reports `pool`'s open and idle connections and its limit on every export.
pub fn observe(pool: &Pool<Postgres>) {
    let pool = pool.clone();
    opentelemetry::global::meter(crate::config::SERVICE_NAME)
        .u64_observable_gauge("db_pool_connections")
        .with_description("Database pool connections: open, idle, and the pool's max")
        .with_callback(move |observer| {
            observer.observe(u64::from(pool.size()), &[KeyValue::new("state", "open")]);
            observer.observe(pool.num_idle() as u64, &[KeyValue::new("state", "idle")]);
            observer.observe(
                u64::from(pool.options().get_max_connections()),
                &[KeyValue::new("state", "max")],
            );
        })
        .build();
}

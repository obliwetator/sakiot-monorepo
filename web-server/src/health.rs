//! Liveness is independent of dependencies; readiness reflects workers and
//! persisted queues that must be drained before this instance serves traffic.

use std::{sync::OnceLock, time::Duration};

use actix_web::{HttpResponse, get, web};
use opentelemetry::{KeyValue, metrics::Gauge};
use serde::Serialize;
use sqlx::{Pool, Postgres};
use tokio::task::AbortHandle;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
// A single permitted render may take 30 minutes. Avoid ejecting an otherwise
// healthy instance merely because accepted work waits behind one such render.
const MAX_QUEUE_AGE_SECONDS: i64 = 60 * 60;
const MAX_ARCHIVE_AGE_SECONDS: i64 = 60 * 60;

#[derive(Clone)]
pub struct HealthState {
    pub workers: Vec<(&'static str, AbortHandle)>,
    pub archive_enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct QueueHealth {
    pub media_queued: i64,
    pub media_running: i64,
    pub composition_queued: i64,
    pub composition_running: i64,
    pub deletion_queued: i64,
    pub deletion_running: i64,
    pub deletion_failed: i64,
    pub archive_pending: i64,
    pub archive_uploading: i64,
    pub archive_problem: i64,
    pub oldest_media_queued_seconds: i64,
    pub oldest_composition_queued_seconds: i64,
    pub oldest_deletion_queued_seconds: i64,
    pub oldest_archive_pending_seconds: i64,
    pub expired_media_leases: i64,
    pub expired_composition_leases: i64,
    pub expired_deletion_leases: i64,
}

impl QueueHealth {
    fn unhealthy(&self, archive_enabled: bool) -> bool {
        self.oldest_media_queued_seconds > MAX_QUEUE_AGE_SECONDS
            || self.oldest_composition_queued_seconds > MAX_QUEUE_AGE_SECONDS
            || self.oldest_deletion_queued_seconds > MAX_QUEUE_AGE_SECONDS
            || self.expired_media_leases > 0
            || self.expired_composition_leases > 0
            || self.expired_deletion_leases > 0
            || self.deletion_failed > 0
            || (archive_enabled && self.oldest_archive_pending_seconds > MAX_ARCHIVE_AGE_SECONDS)
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    database: &'static str,
    release_id: String,
    failed_workers: Vec<&'static str>,
    queues: Option<QueueHealth>,
}

fn release_id() -> String {
    std::env::var("RELEASE_ID").unwrap_or_else(|_| "development".to_string())
}

fn failed_workers(state: &HealthState) -> Vec<&'static str> {
    state
        .workers
        .iter()
        .filter_map(|(name, handle)| handle.is_finished().then_some(*name))
        .collect()
}

async fn queue_health(pool: &Pool<Postgres>) -> Result<QueueHealth, sqlx::Error> {
    sqlx::query_as!(
        QueueHealth,
        r#"SELECT
            (SELECT count(*) FROM media_jobs WHERE state='queued') AS "media_queued!",
            (SELECT count(*) FROM media_jobs WHERE state='running') AS "media_running!",
            (SELECT count(*) FROM composition_jobs WHERE state='queued') AS "composition_queued!",
            (SELECT count(*) FROM composition_jobs WHERE state='running') AS "composition_running!",
            (SELECT count(*) FROM recording_deletion_jobs WHERE state='queued') AS "deletion_queued!",
            (SELECT count(*) FROM recording_deletion_jobs WHERE state='running') AS "deletion_running!",
            (SELECT count(*) FROM recording_deletion_jobs WHERE state='failed') AS "deletion_failed!",
            (SELECT count(*) FROM media_objects WHERE state='pending') AS "archive_pending!",
            (SELECT count(*) FROM media_objects WHERE state='uploading') AS "archive_uploading!",
            (SELECT count(*) FROM media_objects WHERE state IN ('missing','conflict')) AS "archive_problem!",
            (SELECT COALESCE(EXTRACT(EPOCH FROM now()-min(created_at)),0)::bigint FROM media_jobs WHERE state='queued' AND retry_at <= now()) AS "oldest_media_queued_seconds!",
            (SELECT COALESCE(EXTRACT(EPOCH FROM now()-min(created_at)),0)::bigint FROM composition_jobs WHERE state='queued' AND retry_at <= now()) AS "oldest_composition_queued_seconds!",
            (SELECT COALESCE(EXTRACT(EPOCH FROM now()-min(created_at)),0)::bigint FROM recording_deletion_jobs WHERE state='queued' AND retry_at <= now()) AS "oldest_deletion_queued_seconds!",
            (SELECT COALESCE(EXTRACT(EPOCH FROM now()-min(created_at)),0)::bigint FROM media_objects WHERE state IN ('pending','uploading') AND retry_at <= now()) AS "oldest_archive_pending_seconds!",
            (SELECT count(*) FROM media_jobs WHERE state='running' AND lease_expires_at < now()-interval '2 minutes') AS "expired_media_leases!",
            (SELECT count(*) FROM composition_jobs WHERE state='running' AND lease_expires_at < now()-interval '2 minutes') AS "expired_composition_leases!",
            (SELECT count(*) FROM recording_deletion_jobs WHERE state='running' AND lease_expires_at < now()-interval '2 minutes') AS "expired_deletion_leases!""#
    )
    .fetch_one(pool)
    .await
}

struct HealthMetrics {
    queued: Gauge<u64>,
    running: Gauge<u64>,
    oldest_seconds: Gauge<u64>,
    failed_workers: Gauge<u64>,
    ready: Gauge<u64>,
}

fn metrics() -> &'static HealthMetrics {
    static METRICS: OnceLock<HealthMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = opentelemetry::global::meter(crate::telemetry::SERVICE_NAME);
        HealthMetrics {
            queued: meter.u64_gauge("media_queue_queued_jobs").build(),
            running: meter.u64_gauge("media_queue_active_processes").build(),
            oldest_seconds: meter.u64_gauge("media_queue_oldest_age_seconds").build(),
            failed_workers: meter.u64_gauge("media_worker_failed").build(),
            ready: meter.u64_gauge("web_server_ready").build(),
        }
    })
}

fn record_metrics(queues: &QueueHealth, failed: &[&'static str], ready: bool) {
    let m = metrics();
    for (name, queued, running, oldest) in [
        (
            "media",
            queues.media_queued,
            queues.media_running,
            queues.oldest_media_queued_seconds,
        ),
        (
            "composition",
            queues.composition_queued,
            queues.composition_running,
            queues.oldest_composition_queued_seconds,
        ),
        (
            "recording_deletion",
            queues.deletion_queued,
            queues.deletion_running,
            queues.oldest_deletion_queued_seconds,
        ),
        (
            "archive",
            queues.archive_pending,
            queues.archive_uploading,
            queues.oldest_archive_pending_seconds,
        ),
    ] {
        let label = [KeyValue::new("queue", name)];
        m.queued.record(queued.max(0) as u64, &label);
        m.running.record(running.max(0) as u64, &label);
        m.oldest_seconds.record(oldest.max(0) as u64, &label);
    }
    m.failed_workers.record(failed.len() as u64, &[]);
    m.ready.record(u64::from(ready), &[]);
}

async fn probe(pool: &Pool<Postgres>, state: &HealthState) -> HealthResponse {
    let failed = failed_workers(state);
    match tokio::time::timeout(PROBE_TIMEOUT, queue_health(pool)).await {
        Ok(Ok(queues)) => {
            let ready = failed.is_empty() && !queues.unhealthy(state.archive_enabled);
            record_metrics(&queues, &failed, ready);
            HealthResponse {
                status: if ready { "ok" } else { "unavailable" },
                database: "ready",
                release_id: release_id(),
                failed_workers: failed,
                queues: Some(queues),
            }
        }
        result => {
            tracing::warn!(?result, "readiness database probe failed");
            metrics().ready.record(0, &[]);
            HealthResponse {
                status: "unavailable",
                database: "unavailable",
                release_id: release_id(),
                failed_workers: failed,
                queues: None,
            }
        }
    }
}

#[get("/livez")]
pub async fn livez() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "alive", "release_id": release_id()}))
}

async fn readiness_response(
    pool: web::Data<Pool<Postgres>>,
    state: web::Data<HealthState>,
) -> HttpResponse {
    let health = probe(pool.get_ref(), state.get_ref()).await;
    if health.status == "ok" {
        HttpResponse::Ok().json(health)
    } else {
        HttpResponse::ServiceUnavailable().json(health)
    }
}

#[get("/readyz")]
pub async fn readyz(
    pool: web::Data<Pool<Postgres>>,
    state: web::Data<HealthState>,
) -> HttpResponse {
    readiness_response(pool, state).await
}

// Existing deployment checks keep using /healthz as a readiness alias.
#[get("/healthz")]
pub async fn healthz(
    pool: web::Data<Pool<Postgres>>,
    state: web::Data<HealthState>,
) -> HttpResponse {
    readiness_response(pool, state).await
}

pub fn spawn_monitor(pool: Pool<Postgres>, state: HealthState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        let mut was_ready = true;
        loop {
            ticker.tick().await;
            let health = probe(&pool, &state).await;
            let ready = health.status == "ok";
            if !ready && was_ready {
                tracing::error!(failed_workers = ?health.failed_workers, queues = ?health.queues,
                    "readiness became unhealthy; alert on web_server_ready=0");
            } else if ready && !was_ready {
                tracing::info!("readiness recovered");
            }
            was_ready = ready;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, http::StatusCode, test as actix_test};

    #[actix_web::test]
    async fn backlog_and_failed_worker_change_readiness() {
        let mut queues = QueueHealth::default();
        assert!(!queues.unhealthy(false));
        queues.oldest_media_queued_seconds = MAX_QUEUE_AGE_SECONDS + 1;
        assert!(queues.unhealthy(false));
        queues = QueueHealth::default();
        queues.oldest_deletion_queued_seconds = MAX_QUEUE_AGE_SECONDS + 1;
        assert!(queues.unhealthy(false));
        queues = QueueHealth::default();
        queues.deletion_failed = 1;
        assert!(queues.unhealthy(false));
        queues = QueueHealth::default();
        queues.oldest_archive_pending_seconds = MAX_ARCHIVE_AGE_SECONDS + 1;
        assert!(!queues.unhealthy(false));
        assert!(queues.unhealthy(true));
        let worker = tokio::spawn(async {});
        let abort = worker.abort_handle();
        worker.await.unwrap();
        let state = HealthState {
            workers: vec![("media", abort)],
            archive_enabled: false,
        };
        assert_eq!(failed_workers(&state), vec!["media"]);
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn stale_media_jobs_are_visible_to_readiness(
        pool: Pool<Postgres>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO media_jobs (id,kind,user_id,idempotency_key,resource_key,request,created_at) VALUES ('stale','session_mix',1,'stale','stale','{}',now()-interval '61 minutes')")
            .execute(&pool).await?;
        let queues = queue_health(&pool).await?;
        assert_eq!(queues.media_queued, 1);
        assert!(queues.oldest_media_queued_seconds >= 61 * 60);
        assert!(queues.unhealthy(false));
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn readiness_reacts_to_backlog_and_worker_exit(
        pool: Pool<Postgres>,
    ) -> Result<(), sqlx::Error> {
        let worker = tokio::spawn(std::future::pending::<()>());
        let state = HealthState {
            workers: vec![("media", worker.abort_handle())],
            archive_enabled: false,
        };
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(pool.clone()))
                .app_data(web::Data::new(state))
                .service(livez)
                .service(readyz)
                .service(healthz),
        )
        .await;
        let status = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/readyz").to_request(),
        )
        .await
        .status();
        assert_eq!(status, StatusCode::OK);

        sqlx::query("INSERT INTO media_jobs (id,kind,user_id,idempotency_key,resource_key,request,created_at) VALUES ('stale','session_mix',1,'stale','stale','{}',now()-interval '61 minutes')")
            .execute(&pool).await?;
        let status = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/healthz").to_request(),
        )
        .await
        .status();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        sqlx::query("DELETE FROM media_jobs WHERE id='stale'")
            .execute(&pool)
            .await?;

        worker.abort();
        let _ = worker.await;
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/readyz").to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["failed_workers"], serde_json::json!(["media"]));
        let status = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/livez").to_request(),
        )
        .await
        .status();
        assert_eq!(status, StatusCode::OK);
        Ok(())
    }
}

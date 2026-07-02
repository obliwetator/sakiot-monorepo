use actix_web::{HttpResponse, get, web};
use serde::Serialize;
use sqlx::{Pool, Postgres};
use std::time::Duration;

#[derive(Serialize)]
struct HealthResponse<'a> {
    status: &'a str,
    database: &'a str,
    release_id: &'a str,
}

#[get("/healthz")]
pub async fn healthz(pool: web::Data<Pool<Postgres>>) -> HttpResponse {
    let release_id = std::env::var("RELEASE_ID").unwrap_or_else(|_| "development".to_string());
    let probe = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool.get_ref());
    match tokio::time::timeout(Duration::from_secs(2), probe).await {
        Ok(Ok(_)) => HttpResponse::Ok().json(HealthResponse {
            status: "ok",
            database: "ready",
            release_id: &release_id,
        }),
        result => {
            tracing::warn!(?result, "health check database probe failed");
            HttpResponse::ServiceUnavailable().json(HealthResponse {
                status: "unavailable",
                database: "unavailable",
                release_id: &release_id,
            })
        }
    }
}

use std::net::IpAddr;
use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};
use subtle::ConstantTimeEq;
use tracing::{info, warn};

use crate::{config::Config, errors::AppError};

const REGISTRY_SECRET_HEADER: &str = "X-FBI-Agent-Registry-Secret";

#[derive(Clone)]
pub struct AgentGrpcRegistry {
    state: Arc<RwLock<AgentGrpcState>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentGrpcState {
    pub active: String,
    pub draining: Vec<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterAgentGrpcRequest {
    pub active: String,
    #[serde(default)]
    pub draining: Vec<String>,
}

impl AgentGrpcRegistry {
    pub fn new(initial_active: &str) -> Self {
        Self {
            state: Arc::new(RwLock::new(AgentGrpcState {
                active: normalize_grpc_address(initial_active),
                draining: Vec::new(),
                updated_at: None,
            })),
        }
    }

    pub fn active_address(&self) -> String {
        self.state.read().active.clone()
    }

    fn snapshot(&self) -> AgentGrpcState {
        self.state.read().clone()
    }

    fn register(&self, req: RegisterAgentGrpcRequest) -> AgentGrpcState {
        let state = AgentGrpcState {
            active: normalize_grpc_address(&req.active),
            draining: req
                .draining
                .iter()
                .filter(|addr| !addr.trim().is_empty())
                .map(|addr| normalize_grpc_address(addr))
                .collect(),
            updated_at: Some(Utc::now()),
        };
        *self.state.write() = state.clone();
        state
    }
}

/// Address of the agent holding `guild_id`'s voice connection.
///
/// During a blue/green handoff the draining instance keeps the voice connection it
/// already had, so the newly active instance cannot play into that channel. The
/// `voice_session_leases` row records who actually owns the guild — the same signal
/// `should_skip_interaction` uses on the bot to route slash commands.
///
/// Every agent heartbeats its own `GRPC_ADDR` into `bot_instances` every 10s, so this
/// resolves the current port even though the deploy engine hands each release a fresh
/// one. The in-memory registry is only the last resort: it is empty after a web server
/// restart until the next deploy republishes it.
pub async fn agent_address_for_guild(
    pool: &Pool<Postgres>,
    registry: &AgentGrpcRegistry,
    guild_id: i64,
) -> String {
    let address = match agent_grpc_address_for_guild(pool, guild_id).await {
        Ok(address) => address,
        Err(err) => {
            warn!(
                guild_id,
                "failed to resolve the agent owning this guild; falling back to the registry: {}",
                err,
            );
            None
        }
    };

    resolve_agent_address(address.as_deref(), &registry.active_address())
}

/// Pure routing decision, split out so the fallback is testable without a database.
fn resolve_agent_address(resolved_address: Option<&str>, registry_address: &str) -> String {
    match resolved_address {
        Some(address) if !address.trim().is_empty() => normalize_grpc_address(address),
        _ => registry_address.to_string(),
    }
}

/// The guild's voice lease owner, else the newest live active instance. Both come from
/// the agents' own heartbeats, so neither depends on the deploy engine having published
/// anything to this process.
async fn agent_grpc_address_for_guild(
    pool: &Pool<Postgres>,
    guild_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT COALESCE(
                  (SELECT b.grpc_address
                     FROM voice_session_leases v
                     JOIN bot_instances b ON b.instance_id = v.owner_instance_id
                    WHERE v.guild_id = $1
                      AND v.heartbeat_at > now() - interval '120 seconds'
                      AND b.heartbeat_at > now() - interval '120 seconds'
                      AND b.state <> 'stopped'
                      AND b.grpc_address IS NOT NULL
                    LIMIT 1),
                  (SELECT b.grpc_address
                     FROM bot_instances b
                    WHERE b.state = 'active'
                      AND b.heartbeat_at > now() - interval '120 seconds'
                      AND b.grpc_address IS NOT NULL
                    ORDER BY b.started_at DESC
                    LIMIT 1)
                )",
        guild_id
    )
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
}

#[post("/internal/fbi-agent/grpc-endpoints")]
pub async fn register_agent_grpc_endpoints(
    req: HttpRequest,
    body: web::Json<RegisterAgentGrpcRequest>,
    cfg: web::Data<Config>,
    registry: web::Data<AgentGrpcRegistry>,
) -> Result<HttpResponse, AppError> {
    if !authorized_internal_request(&req, &cfg) {
        return Err(AppError::Unauthorized);
    }

    if body.active.trim().is_empty() {
        return Err(AppError::BadRequest(
            "active gRPC address is required".into(),
        ));
    }

    let state = registry.register(body.into_inner());
    info!(
        active = %state.active,
        draining = ?state.draining,
        "registered FBI agent gRPC endpoints"
    );
    Ok(HttpResponse::Ok().json(state))
}

#[get("/internal/fbi-agent/grpc-endpoints")]
pub async fn get_agent_grpc_endpoints(
    req: HttpRequest,
    cfg: web::Data<Config>,
    registry: web::Data<AgentGrpcRegistry>,
) -> Result<HttpResponse, AppError> {
    if !authorized_internal_request(&req, &cfg) {
        return Err(AppError::Unauthorized);
    }

    Ok(HttpResponse::Ok().json(registry.snapshot()))
}

fn authorized_internal_request(req: &HttpRequest, cfg: &Config) -> bool {
    req.peer_addr()
        .map(|addr| addr.ip())
        .is_some_and(is_loopback_ip)
        || cfg
            .fbi_agent_registry_secret
            .as_ref()
            .is_some_and(|expected| header_matches(req, expected))
}

fn header_matches(req: &HttpRequest, expected: &str) -> bool {
    let Some(actual) = req
        .headers()
        .get(REGISTRY_SECRET_HEADER)
        .and_then(|header| header.to_str().ok())
    else {
        return false;
    };

    actual.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn normalize_grpc_address(address: &str) -> String {
    let trimmed = address.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentGrpcRegistry, agent_grpc_address_for_guild, resolve_agent_address};
    use sqlx::PgPool;

    const GUILD: i64 = 4242;

    /// Mid-handoff: the old instance is draining but still owns the guild's voice
    /// connection, and the new one is active with nothing to play into.
    async fn seed_handoff(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO bot_instances (instance_id, role, state, grpc_address, heartbeat_at, started_at)
             VALUES
                ('old', 'drain', 'draining', '127.0.0.1:40000', now(), now() - interval '1 hour'),
                ('new', 'active', 'active', '127.0.0.1:40123', now(), now())",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO voice_session_leases
                (guild_id, channel_id, owner_instance_id, state, heartbeat_at, started_at)
             VALUES ($1, 99, 'old', 'draining', now(), now())",
        )
        .bind(GUILD)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[test]
    fn registry_normalizes_host_port_addresses() {
        let registry = AgentGrpcRegistry::new("127.0.0.1:59877");

        assert_eq!(registry.active_address(), "http://127.0.0.1:59877");
    }

    #[test]
    fn heartbeated_address_wins_over_the_registry() {
        assert_eq!(
            resolve_agent_address(Some("127.0.0.1:40000"), "http://127.0.0.1:40123"),
            "http://127.0.0.1:40000"
        );
    }

    #[test]
    fn falls_back_to_the_registry_without_a_usable_address() {
        let registry = "http://127.0.0.1:40123";

        assert_eq!(resolve_agent_address(None, registry), registry);
        assert_eq!(resolve_agent_address(Some("   "), registry), registry);
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn draining_lease_owner_beats_the_active_instance(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_handoff(&pool).await?;

        assert_eq!(
            agent_grpc_address_for_guild(&pool, GUILD).await?,
            Some("127.0.0.1:40000".to_owned())
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn guild_without_a_lease_uses_the_newest_active_instance(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_handoff(&pool).await?;

        assert_eq!(
            agent_grpc_address_for_guild(&pool, GUILD + 1).await?,
            Some("127.0.0.1:40123".to_owned())
        );
        Ok(())
    }

    /// A lease left behind by an instance that died is not a routing target — the
    /// drain finished, so the active instance owns the guild now.
    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn stale_lease_owner_is_ignored(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
        seed_handoff(&pool).await?;
        sqlx::query("UPDATE bot_instances SET state = 'stopped' WHERE instance_id = 'old'")
            .execute(&pool)
            .await?;

        assert_eq!(
            agent_grpc_address_for_guild(&pool, GUILD).await?,
            Some("127.0.0.1:40123".to_owned())
        );

        sqlx::query(
            "UPDATE bot_instances
                SET state = 'draining', heartbeat_at = now() - interval '10 minutes'
              WHERE instance_id = 'old'",
        )
        .execute(&pool)
        .await?;

        assert_eq!(
            agent_grpc_address_for_guild(&pool, GUILD).await?,
            Some("127.0.0.1:40123".to_owned())
        );
        Ok(())
    }

    /// An instance that predates `bot_instances.grpc_address` cannot be dialled, so
    /// routing has to fall through rather than return NULL.
    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn lease_owner_without_an_address_falls_through(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        seed_handoff(&pool).await?;
        sqlx::query("UPDATE bot_instances SET grpc_address = NULL WHERE instance_id = 'old'")
            .execute(&pool)
            .await?;

        assert_eq!(
            agent_grpc_address_for_guild(&pool, GUILD).await?,
            Some("127.0.0.1:40123".to_owned())
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn no_live_instance_resolves_to_nothing(
        pool: PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(agent_grpc_address_for_guild(&pool, GUILD).await?, None);
        Ok(())
    }
}

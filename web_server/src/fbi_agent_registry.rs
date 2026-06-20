use std::net::IpAddr;
use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tracing::info;

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
    use super::AgentGrpcRegistry;

    #[test]
    fn registry_normalizes_host_port_addresses() {
        let registry = AgentGrpcRegistry::new("127.0.0.1:59877");

        assert_eq!(registry.active_address(), "http://127.0.0.1:59877");
    }
}

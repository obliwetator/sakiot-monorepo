pub mod fbi_agent {
    tonic::include_proto!("fbi_agent");
}

/// Header, and gRPC metadata key, that carries an environment's
/// `FBI_AGENT_REGISTRY_SECRET`. Every call to the agent's gRPC services and to
/// web-server's `/internal/fbi-agent/grpc-endpoints` presents it: production,
/// staging and every preview slot share the host's loopback, so reaching a port
/// proves nothing about the caller. Lowercase, as gRPC metadata keys must be.
pub const INTERNAL_SECRET_HEADER: &str = "x-fbi-agent-registry-secret";

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::{env, error::Error};

pub struct DiscordConfig {
    pub token: String,
    pub application_id: u64,
}

pub fn db_url() -> Result<String, env::VarError> {
    env::var("DATABASE_URL")
}

#[cfg(debug_assertions)]
pub fn discord_config() -> Result<DiscordConfig, Box<dyn Error + Send + Sync>> {
    Ok(DiscordConfig {
        token: env::var("DISCORD_TOKEN_DEBUG")?,
        application_id: env::var("APPLICATION_ID_DEBUG")?.parse()?,
    })
}

#[cfg(not(debug_assertions))]
pub fn discord_config() -> Result<DiscordConfig, Box<dyn Error + Send + Sync>> {
    Ok(DiscordConfig {
        token: env::var("DISCORD_TOKEN_RELEASE")?,
        application_id: env::var("APPLICATION_ID_RELEASE")?.parse()?,
    })
}

pub fn grpc_addr() -> String {
    if let Ok(addr) = env::var("GRPC_ADDR") {
        return addr;
    }

    #[cfg(debug_assertions)]
    {
        "[::1]:50053".to_string()
    }
    #[cfg(not(debug_assertions))]
    {
        "[::1]:50052".to_string()
    }
}

/// The address other services dial to reach this instance's gRPC server, published to
/// `bot_instances.grpc_address` so the web server can route a jam to whichever instance
/// owns a guild's voice connection.
///
/// `GRPC_ADDR` is a *bind* address — the deploy engine hands each release a fresh
/// `127.0.0.1:<free port>` — so a wildcard bind has to be rewritten before it is
/// published. `None` when it cannot be parsed; the gRPC server would already have
/// failed to bind, and the column simply stays NULL.
pub fn grpc_dial_addr() -> Option<String> {
    dial_addr_from_bind(&grpc_addr())
}

fn dial_addr_from_bind(bind: &str) -> Option<String> {
    let addr: SocketAddr = bind.parse().ok()?;
    if !addr.ip().is_unspecified() {
        return Some(addr.to_string());
    }

    let loopback = match addr.ip() {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    Some(SocketAddr::new(loopback, addr.port()).to_string())
}

#[cfg(debug_assertions)]
pub const SERVICE_NAME: &str = "fbi-agent-debug";
#[cfg(not(debug_assertions))]
pub const SERVICE_NAME: &str = "fbi-agent";

#[cfg(test)]
mod tests {
    use super::dial_addr_from_bind;

    #[test]
    fn dialable_binds_pass_through() {
        assert_eq!(
            dial_addr_from_bind("127.0.0.1:41337").as_deref(),
            Some("127.0.0.1:41337")
        );
        assert_eq!(
            dial_addr_from_bind("[::1]:50053").as_deref(),
            Some("[::1]:50053")
        );
    }

    #[test]
    fn wildcard_binds_become_loopback() {
        assert_eq!(
            dial_addr_from_bind("0.0.0.0:50052").as_deref(),
            Some("127.0.0.1:50052")
        );
        assert_eq!(
            dial_addr_from_bind("[::]:50052").as_deref(),
            Some("[::1]:50052")
        );
    }

    #[test]
    fn unparseable_binds_have_no_dial_address() {
        assert_eq!(dial_addr_from_bind("not-an-address"), None);
        assert_eq!(dial_addr_from_bind(""), None);
    }
}

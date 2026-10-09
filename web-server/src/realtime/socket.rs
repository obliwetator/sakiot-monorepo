//! `GET /api/realtime`: the WebSocket a dashboard keeps open for refresh
//! signals.

use std::sync::Arc;
use std::time::Duration;

use actix_web::{HttpRequest, HttpResponse, get, web};
use actix_ws::{AggregatedMessage, CloseCode, CloseReason};

use super::hub::{Connection, Hub};
use super::protocol::{
    CLOSE_TOKEN_EXPIRED, CLOSE_UNSUPPORTED_VERSION, ClientMessage, PROTOCOL_VERSION, ParseError,
    ServerMessage, parse_client_message,
};
use crate::auth::{Access, Token};
use crate::config::Config;
use crate::errors::AppError;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
/// Without a pong or any client message for this long, the client is gone.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Client messages are tiny (`set_scope`, `heartbeat`).
const MAX_MESSAGE_BYTES: usize = 4 * 1024;

/// Only the frontend's own origins may open the socket. Browsers send cookies
/// with cross-site WebSocket handshakes and actix-cors does not block a
/// disallowed origin on a non-preflight request, so this check is what stops
/// another site from riding the viewer's session. Unlike the CORS check it is
/// exact: no subdomain wildcard.
pub fn origin_allowed(origin: Option<&str>, config: &Config) -> bool {
    let Some(origin) = origin else {
        return false;
    };
    let well_formed = origin.parse::<actix_web::http::Uri>().is_ok_and(|uri| {
        matches!(uri.scheme_str(), Some("http" | "https")) && uri.authority().is_some()
    });
    if !well_formed {
        return false;
    }
    origin == config.cors_allowed_origin
        || config
            .oauth_allowed_opener_origins
            .iter()
            .any(|allowed| allowed == origin)
}

#[utoipa::path(
    get,
    path = "/api/realtime",
    tag = "realtime",
    responses(
        (status = 101, description = "WebSocket upgrade; see the realtime protocol schemas"),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Origin not allowed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/realtime")]
pub async fn realtime_socket(
    req: HttpRequest,
    body: web::Payload,
    token: Option<web::ReqData<Token<Access>>>,
    config: web::Data<Config>,
    hub: web::Data<Hub>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?.into_inner();
    let origin = req
        .headers()
        .get(actix_web::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !origin_allowed(origin, &config) {
        tracing::warn!(?origin, "realtime connection from a disallowed origin");
        return Err(AppError::Forbidden);
    }

    let (response, session, stream) = actix_ws::handle(&req, body)
        .map_err(|_| AppError::BadRequest("websocket upgrade".into()))?;
    let stream = stream
        .max_frame_size(MAX_MESSAGE_BYTES)
        .aggregate_continuations()
        .max_continuation_size(MAX_MESSAGE_BYTES);

    let hub = hub.into_inner();
    let connection = hub.register(crate::permissions::Viewer::of(&token));
    actix_web::rt::spawn(run(hub, connection, token, session, stream));
    Ok(response)
}

fn close(code: u16, description: &str) -> Option<CloseReason> {
    Some(CloseReason {
        code: CloseCode::Other(code),
        description: Some(description.to_string()),
    })
}

async fn run(
    hub: Arc<Hub>,
    connection: Arc<Connection>,
    token: Token<Access>,
    session: actix_ws::Session,
    stream: actix_ws::AggregatedMessageStream,
) {
    let reason = serve(&hub, &connection, &token, session.clone(), stream).await;
    hub.unregister(connection.id);
    let _ = session.close(reason).await;
}

/// Runs the connection until it should close; returns the close reason.
async fn serve(
    hub: &Hub,
    connection: &Connection,
    token: &Token<Access>,
    mut session: actix_ws::Session,
    mut stream: actix_ws::AggregatedMessageStream,
) -> Option<CloseReason> {
    let expires_at_ms =
        i64::try_from(token.exp.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX);
    let until_expiry = Duration::from_millis(
        u64::try_from(expires_at_ms - chrono::Utc::now().timestamp_millis()).unwrap_or(0),
    );
    let expiry = tokio::time::sleep(until_expiry);
    tokio::pin!(expiry);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;
    let mut last_seen = tokio::time::Instant::now();

    connection.push(ServerMessage::Ready {
        v: PROTOCOL_VERSION,
        user_id: token.user_id.to_string(),
        server_time: chrono::Utc::now().timestamp_millis(),
        token_expires_at: expires_at_ms,
    });

    loop {
        // Write whatever is queued first; a requested close ends the loop.
        let (messages, close_code) = connection.take();
        for message in messages {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if session.text(text).await.is_err() {
                return None;
            }
        }
        if let Some(code) = close_code {
            return close(code, "server closing");
        }

        tokio::select! {
            () = connection.wait() => {}
            () = &mut expiry => return close(CLOSE_TOKEN_EXPIRED, "access token expired"),
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > CLIENT_TIMEOUT {
                    return Some(CloseCode::Away.into());
                }
                if session.ping(b"").await.is_err() {
                    return None;
                }
                connection.push(ServerMessage::Heartbeat { v: PROTOCOL_VERSION });
            }
            message = stream.recv() => {
                let Some(Ok(message)) = message else {
                    return None;
                };
                last_seen = tokio::time::Instant::now();
                match message {
                    AggregatedMessage::Text(text) => match parse_client_message(&text) {
                        Ok(ClientMessage::SetScope {
                            guild_id,
                            as_role,
                            presence_updates,
                            ..
                        }) => {
                            let Ok(guild_id) = guild_id.parse::<i64>() else {
                                return Some(CloseCode::Invalid.into());
                            };
                            let as_role = match as_role.map(|role| role.parse::<i64>()) {
                                None => None,
                                Some(Ok(role)) => Some(role),
                                Some(Err(_)) => return Some(CloseCode::Invalid.into()),
                            };
                            hub.set_scope(connection, guild_id, as_role, presence_updates)
                                .await;
                        }
                        Ok(ClientMessage::Heartbeat { .. }) => {}
                        Err(ParseError::UnsupportedVersion) => {
                            return close(CLOSE_UNSUPPORTED_VERSION, "unsupported protocol version");
                        }
                        Err(ParseError::Malformed) => return Some(CloseCode::Invalid.into()),
                    },
                    AggregatedMessage::Ping(bytes) => {
                        if session.pong(&bytes).await.is_err() {
                            return None;
                        }
                    }
                    AggregatedMessage::Pong(_) => {}
                    AggregatedMessage::Binary(_) => return Some(CloseCode::Unsupported.into()),
                    AggregatedMessage::Close(_) => return None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::origin_allowed;
    use crate::config::Config;

    fn config() -> Config {
        Config {
            database_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            access_secret: String::new(),
            refresh_secret: String::new(),
            dev_account_id: 0,
            dev_login_secret: None,
            cors_allowed_origin: "https://debug.example.com".into(),
            oauth_allowed_opener_origins: vec!["https://staging.example.com".into()],
            oauth_allowed_opener_host_suffixes: Vec::new(),
            discord_redirect_uri: String::new(),
            grpc_address: String::new(),
            fbi_agent_registry_secret: None,
            host: String::new(),
            port: 0,
            db_max_connections: 1,
            recording_permanent_delete_enabled: false,
            server_timing_header: false,
        }
    }

    #[test]
    fn only_exact_frontend_origins_may_connect() {
        let config = config();
        assert!(origin_allowed(Some("https://debug.example.com"), &config));
        assert!(origin_allowed(Some("https://staging.example.com"), &config));
        for rejected in [
            None,
            Some("null"),
            Some("not a url"),
            Some("https://evil.debug.example.com"),
            Some("https://debug.example.com.evil.com"),
            Some("http://debug.example.com"),
            Some("https://example.com"),
        ] {
            assert!(!origin_allowed(rejected, &config), "{rejected:?}");
        }
    }
}

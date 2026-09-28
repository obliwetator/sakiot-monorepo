use actix_web::web;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::Config;
use crate::errors::AppError;

pub const BASE_URL: &str = "https://discord.com/api/v10/";

#[derive(Deserialize, Debug)]
pub struct DiscordLoginCode {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Serialize, Debug)]
struct DiscordBotAuthData {
    client_id: String,
    client_secret: String,
    grant_type: &'static str,
    code: String,
    redirect_uri: String,
}

#[derive(Serialize, Debug)]
struct DiscordBotAuthDataRefresh {
    client_id: String,
    client_secret: String,
    grant_type: &'static str,
    refresh_token: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DiscordTokenData {
    pub access_token: String,
    pub expires_in: i32,
    pub refresh_token: String,
    pub scope: String,
    pub token_type: String,
}

/// Discord error bodies are deliberately never passed through to clients.
/// OAuth code rejection is a client-auth failure; rate limiting and upstream
/// failures remain distinct so callers can retry appropriately.
pub async fn parse_discord_response<T: DeserializeOwned>(
    request: RequestBuilder,
    token_exchange: bool,
) -> Result<T, AppError> {
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            AppError::DiscordTimeout
        } else {
            AppError::DiscordUnavailable(error.to_string())
        }
    })?;
    check_discord_status(&response, token_exchange)?;
    response
        .json::<T>()
        .await
        .map_err(|error| AppError::DiscordUnavailable(format!("invalid response body: {error}")))
}

fn check_discord_status(response: &Response, token_exchange: bool) -> Result<(), AppError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        return Err(AppError::DiscordRateLimited { retry_after });
    }
    if status == StatusCode::UNAUTHORIZED || (token_exchange && status == StatusCode::BAD_REQUEST) {
        return Err(AppError::Unauthorized);
    }
    Err(AppError::DiscordUnavailable(format!(
        "Discord returned HTTP {status}"
    )))
}

pub async fn request_access_token(
    cfg: &Config,
    code: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    request_access_token_at(cfg, code, client, BASE_URL).await
}

async fn request_access_token_at(
    cfg: &Config,
    code: String,
    client: web::Data<Client>,
    base_url: &str,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthData {
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        grant_type: "authorization_code",
        code,
        redirect_uri: cfg.discord_redirect_uri.clone(),
    };

    parse_discord_response(
        client.post(format!("{base_url}oauth2/token")).form(&data),
        true,
    )
    .await
}

pub async fn request_refresh_token(
    cfg: &Config,
    refresh_token: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthDataRefresh {
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        grant_type: "refresh_token",
        refresh_token,
    };

    parse_discord_response(
        client.post(format!("{}oauth2/token", BASE_URL)).form(&data),
        true,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::error::ResponseError;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn response_url(status: &str, headers: &str, body: &str, delay_ms: u64) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).await;
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            let _ = stream.write_all(response.as_bytes()).await;
        });
        format!("http://{address}/")
    }

    async fn token_error(status: &str, headers: &str, delay_ms: u64) -> AppError {
        let url = response_url(status, headers, r#"{"error":"invalid_grant"}"#, delay_ms).await;
        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(30))
            .build()
            .unwrap();
        parse_discord_response::<DiscordTokenData>(client.post(url), true)
            .await
            .unwrap_err()
    }

    #[tokio::test]
    async fn invalid_oauth_code_is_unauthorized() {
        assert_eq!(
            token_error("400 Bad Request", "", 0).await.status_code(),
            actix_web::http::StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn rate_limit_keeps_retry_after_without_exposing_body() {
        let error = token_error("429 Too Many Requests", "Retry-After: 7\r\n", 0).await;
        assert!(matches!(
            error,
            AppError::DiscordRateLimited {
                retry_after: Some(7)
            }
        ));
        assert_eq!(
            error.status_code(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn server_failure_and_timeout_are_gateway_errors() {
        assert_eq!(
            token_error("503 Service Unavailable", "", 0)
                .await
                .status_code(),
            actix_web::http::StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            token_error("200 OK", "", 100).await.status_code(),
            actix_web::http::StatusCode::GATEWAY_TIMEOUT
        );
    }
}

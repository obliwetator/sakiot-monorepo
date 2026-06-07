use actix_web::web;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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

pub async fn request_access_token(
    cfg: &Config,
    code: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthData {
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        grant_type: "authorization_code",
        code,
        redirect_uri: cfg.discord_redirect_uri.clone(),
    };

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    Ok(result.json::<DiscordTokenData>().await?)
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

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    Ok(result.json::<DiscordTokenData>().await?)
}

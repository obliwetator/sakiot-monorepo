use actix_web::web;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::{CLIENT_ID, CLIENT_SECRET, DISCORD_REDIRECT_URI};
use crate::errors::AppError;

pub const BASE_URL: &str = "https://discord.com/api/v10/";
pub const BASE_AUTH_URL: &str = "https://discord.com/oauth2/authorize/";
pub const TOKEN_URL: &str = "https://discord.com/api/oauth2/token/";

#[derive(Deserialize, Debug)]
pub struct DiscordLoginCode {
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordBotAuthData {
    client_id: &'static str,
    client_secret: &'static str,
    grant_type: &'static str,
    code: String,
    redirect_uri: &'static str,
}

impl Default for DiscordBotAuthData {
    fn default() -> Self {
        Self {
            client_id: CLIENT_ID.as_str(),
            client_secret: CLIENT_SECRET.as_str(),
            grant_type: "authorization_code",
            code: String::new(),
            redirect_uri: DISCORD_REDIRECT_URI.as_str(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct DiscordBotAuthDataRefresh {
    client_id: &'static str,
    client_secret: &'static str,
    grant_type: &'static str,
    refresh_token: String,
}

impl DiscordBotAuthDataRefresh {
    fn new(refresh_token: String) -> Self {
        Self {
            client_id: CLIENT_ID.as_str(),
            client_secret: CLIENT_SECRET.as_str(),
            grant_type: "refresh_token",
            refresh_token,
        }
    }
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
    code: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthData {
        code,
        ..Default::default()
    };

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    Ok(result.json::<DiscordTokenData>().await?)
}

pub async fn request_refresh_token(
    refresh_token: String,
    client: web::Data<Client>,
) -> Result<DiscordTokenData, AppError> {
    let data = DiscordBotAuthDataRefresh::new(refresh_token);

    let result = client
        .post(format!("{}oauth2/token", BASE_URL))
        .form(&data)
        .send()
        .await?;

    Ok(result.json::<DiscordTokenData>().await?)
}

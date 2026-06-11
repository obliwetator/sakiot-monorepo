//! Web server HTTP client: /healthz readiness checks and the FBI Agent gRPC
//! endpoint registry. Replaces the curl/jq pipelines in deploy-release.sh
//! with matching timeouts (--connect-timeout 2 --max-time 5).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;

pub const REGISTRY_SECRET_HEADER: &str = "X-FBI-Agent-Registry-Secret";

#[derive(Debug, Serialize)]
struct RegistryBody<'a> {
    active: &'a str,
    draining: &'a [String],
}

pub trait WebApi {
    /// `.status == "ok" and .database == "ready" and .release_id == $release`.
    fn health_ready(&self, url: &str, release_id: &str) -> bool;

    fn publish_registry(
        &self,
        url: &str,
        secret: &str,
        active: &str,
        draining: &[String],
    ) -> Result<()>;
}

pub struct ReqwestWebApi {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl ReqwestWebApi {
    pub fn new() -> Result<ReqwestWebApi> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .context("failed to build HTTP client")?;
        Ok(ReqwestWebApi { runtime, client })
    }
}

impl WebApi for ReqwestWebApi {
    fn health_ready(&self, url: &str, release_id: &str) -> bool {
        let response: Result<serde_json::Value> = self.runtime.block_on(async {
            let response = self.client.get(url).send().await?.error_for_status()?;
            Ok(response.json().await?)
        });
        match response {
            Ok(body) => {
                body.get("status").and_then(|v| v.as_str()) == Some("ok")
                    && body.get("database").and_then(|v| v.as_str()) == Some("ready")
                    && body.get("release_id").and_then(|v| v.as_str()) == Some(release_id)
            }
            Err(_) => false,
        }
    }

    fn publish_registry(
        &self,
        url: &str,
        secret: &str,
        active: &str,
        draining: &[String],
    ) -> Result<()> {
        let body = RegistryBody { active, draining };
        let result = self.runtime.block_on(async {
            let mut request = self
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !secret.is_empty() {
                request = request.header(REGISTRY_SECRET_HEADER, secret);
            }
            request.send().await?.error_for_status()?;
            Ok::<(), reqwest::Error>(())
        });
        if let Err(error) = result {
            bail!("registry publish to {url} failed: {error}");
        }
        Ok(())
    }
}

//! `sakiot-deploy status {production|staging}` — read-only snapshot of the
//! deployed state: state files, web health, the bot registry, and per-unit
//! drain status. Local-only, like --dry-run; not reachable through the SSH
//! forced command. Run as a user that can read the release tree (sakiot or
//! root).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::admin_api::AdminApi;
use crate::config::{Config, Target};
use crate::fsx;

pub fn run(target: Target, config: &Config, admin: &dyn AdminApi) -> Result<()> {
    println!("target: {}", target.as_str());
    println!("env file: {}", config.env_file.display());
    println!();

    print_state(config);
    println!();
    print_web(config);
    println!();
    print_registry(config);
    println!();
    print_bot_units(config, admin)
}

fn state_line(config: &Config, name: &str) -> String {
    let path = config.state_dir.join(name);
    match fsx::read_line(&path) {
        Some(value) if !value.is_empty() => value,
        Some(_) => "(empty)".to_string(),
        None if path.exists() => "(unreadable)".to_string(),
        None => "(missing)".to_string(),
    }
}

fn print_state(config: &Config) {
    println!("deploy state ({}):", config.state_dir.display());
    for name in [
        "current.tag",
        "current.sha",
        "current-bot.unit",
        "current-bot.grpc",
        "current.manifest",
    ] {
        println!("  {name}: {}", state_line(config, name));
    }
}

fn http_get_json(url: &str, secret: &str) -> Result<serde_json::Value> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build HTTP client")?;
    runtime.block_on(async {
        let mut request = client.get(url);
        if !secret.is_empty() {
            request = request.header("X-FBI-Agent-Registry-Secret", secret);
        }
        let response = request.send().await.with_context(|| format!("GET {url}"))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("GET {url} returned {status}");
        }
        response
            .json()
            .await
            .with_context(|| format!("GET {url}: body"))
    })
}

fn print_web(config: &Config) {
    println!("web ({}):", config.web_health_url);
    match http_get_json(&config.web_health_url, "") {
        Ok(body) => {
            for key in ["status", "database", "release_id"] {
                let value = body.get(key).and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {key}: {value}");
            }
        }
        Err(error) => println!("  unreachable: {error:#}"),
    }
}

fn print_registry(config: &Config) {
    println!("bot registry ({}):", config.web_registry_url);
    match http_get_json(&config.web_registry_url, &config.registry_secret) {
        Ok(body) => {
            let active = body.get("active").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  active: {active}");
            let draining: Vec<&str> = body
                .get("draining")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            println!(
                "  draining: {}",
                if draining.is_empty() {
                    "(none)".to_string()
                } else {
                    draining.join(", ")
                }
            );
            let updated = body
                .get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("(never since web start)");
            println!("  updated_at: {updated}");
        }
        Err(error) => println!("  unreachable: {error:#}"),
    }
}

/// `systemctl list-units` rows for the bot unit prefix: (unit, active-state).
fn bot_units(prefix: &str) -> Result<Vec<(String, String)>> {
    let output = Command::new("systemctl")
        .args([
            "list-units",
            "--all",
            "--plain",
            "--no-legend",
            "--no-pager",
            &format!("{prefix}*"),
        ])
        .output()
        .context("failed to run systemctl list-units")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut units = Vec::new();
    for line in stdout.lines() {
        let mut fields = line.split_whitespace();
        let Some(unit) = fields.next() else { continue };
        // Columns: UNIT LOAD ACTIVE SUB DESCRIPTION...
        let active = fields.nth(1).unwrap_or("?").to_string();
        units.push((unit.to_string(), active));
    }
    units.sort();
    Ok(units)
}

fn grpc_addr_for_release(release_root: &Path, release_id: &str) -> Option<String> {
    let env_path = release_root
        .join(release_id)
        .join("fbi-agent")
        .join("service.env");
    let content = std::fs::read_to_string(env_path).ok()?;
    content
        .lines()
        .find_map(|line| line.strip_prefix("GRPC_ADDR="))
        .map(str::to_string)
}

fn print_bot_units(config: &Config, admin: &dyn AdminApi) -> Result<()> {
    println!("bot units ({}*):", config.bot_unit_prefix);
    let units = bot_units(&config.bot_unit_prefix)?;
    if units.is_empty() {
        println!("  (none)");
        return Ok(());
    }
    for (unit, active) in units {
        println!("  {unit}: {active}");
        let release_id = unit
            .strip_prefix(&config.bot_unit_prefix)
            .unwrap_or(&unit)
            .strip_suffix(".service")
            .unwrap_or(&unit);
        let Some(address) = grpc_addr_for_release(&config.release_root, release_id) else {
            println!("    grpc: (no readable service.env)");
            continue;
        };
        match admin.drain_status(&address) {
            Ok(status) => {
                println!("    grpc: {address}");
                println!(
                    "    role: {} | draining: {} | shutdown_when_empty: {}",
                    status.role, status.draining, status.shutdown_when_empty
                );
                println!(
                    "    voice connections: {} | recordings: {} | drain age: {}s",
                    status.active_voice_connections,
                    status.active_recordings,
                    status.drain_age_seconds
                );
                if !status.message.is_empty() {
                    println!("    message: {}", status.message);
                }
            }
            Err(error) => println!("    grpc {address}: unreachable ({error:#})"),
        }
    }
    Ok(())
}

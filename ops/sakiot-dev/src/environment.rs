//! Atomic local environment generation and media directory preparation.

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

use crate::config::{Config, DEFAULT_LOCAL_DATABASE_URL, DEFAULT_TEST_DATABASE_URL};

pub fn ensure_local_environment(root: &Path, config: &Config) -> Result<()> {
    let env_path = root.join(".env");
    let mut generated_env = false;
    if !env_path.exists() {
        let example = root.join(".env.example");
        let template = fs::read_to_string(&example)
            .with_context(|| format!("could not read {}", example.display()))?;
        let port = choose_local_port()?;
        let access_secret = secure_hex(32)?;
        let refresh_secret = secure_hex(32)?;
        let dev_secret = secure_hex(16)?;
        let data_dir = root.join("data");
        let rendered = render_env(
            &template,
            &[
                ("DATABASE_URL", DEFAULT_LOCAL_DATABASE_URL.to_string()),
                (
                    "SAKIOT_TEST_DATABASE_URL",
                    DEFAULT_TEST_DATABASE_URL.to_string(),
                ),
                ("SAKIOT_DATA_DIR", data_dir.display().to_string()),
                ("JWT_ACCESS_SECRET", access_secret),
                ("JWT_REFRESH_SECRET", refresh_secret),
                ("DEV_ACCOUNT_ID", config.dev_account_id.to_string()),
                ("DEV_LOGIN_SECRET", dev_secret.clone()),
                ("VITE_DEV_LOGIN_SECRET", dev_secret),
                ("PORT", port.to_string()),
            ],
            port,
        );
        atomic_write_private(&env_path, rendered.as_bytes())?;
        generated_env = true;
        log(format!(
            "wrote {} with secure local development secrets",
            env_path.display()
        ));
    }

    if generated_env {
        dotenvy::from_path(&env_path)
            .with_context(|| format!("could not load generated {}", env_path.display()))?;
    }

    let frontend_env = root.join(".env.development.local");
    if !frontend_env.exists() {
        let port = env_value("PORT")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(config.port);
        let content = format!(
            "VITE_API_URL=http://localhost:{}/api/\nVITE_DEV_LOGIN_SECRET={}\n",
            port,
            env_value("DEV_LOGIN_SECRET").unwrap_or_default()
        );
        atomic_write_private(&frontend_env, content.as_bytes())?;
        log(format!("wrote {}", frontend_env.display()));
    }
    Ok(())
}

pub fn prepare_media_dirs(data_dir: &Path) -> Result<()> {
    for path in [
        data_dir.join("voice_recordings"),
        data_dir.join("no_silence_voice_recordings"),
        data_dir.join("waveform_data"),
        data_dir.join("clips"),
        data_dir.join("voice_recordings/111111111111111111"),
    ] {
        fs::create_dir_all(&path)
            .with_context(|| format!("could not create media directory {}", path.display()))?;
    }
    Ok(())
}

pub fn choose_local_port() -> Result<u16> {
    for port in [8900, 8902, 8903, 8904, 8905] {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    bail!("no free port in 8900-8905; set PORT in .env manually")
}

pub fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn render_env(template: &str, replacements: &[(&str, String)], port: u16) -> String {
    let replacements = replacements
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut output = String::with_capacity(template.len() + 256);
    for line in template.lines() {
        let mut replaced = false;
        for (key, value) in &replacements {
            if line.starts_with(&format!("{key}=")) {
                output.push_str(key);
                output.push('=');
                output.push_str(value);
                output.push('\n');
                replaced = true;
                break;
            }
        }
        if !replaced {
            let rewritten = line
                .replace("localhost:8900", &format!("localhost:{port}"))
                .replace("127.0.0.1:8900", &format!("127.0.0.1:{port}"));
            output.push_str(&rewritten);
            output.push('\n');
        }
    }
    output
}

pub fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("could not create a temporary file in {}", parent.display()))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| {
        anyhow::anyhow!("could not atomically write {}: {error}", path.display())
    })?;
    Ok(())
}

fn secure_hex(bytes: usize) -> Result<String> {
    let mut data = vec![0_u8; bytes];
    getrandom::fill(&mut data).map_err(|error| {
        anyhow::anyhow!("operating system could not provide secure random bytes: {error}")
    })?;
    let mut output = String::with_capacity(bytes * 2);
    for byte in data {
        output.push_str(&format!("{byte:02x}"));
    }
    Ok(output)
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

#[expect(
    clippy::print_stdout,
    reason = "local development progress mirrors the deploy CLI's plain progress output"
)]
fn log(message: String) {
    println!("[dev] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_replaces_values_and_rewrites_local_urls() {
        let rendered = render_env(
            "DATABASE_URL=old\nVITE_API_URL=http://localhost:8900/api/\nWEB_SERVER_URL=http://127.0.0.1:8900\n# localhost:8900 stays in comments too\n",
            &[("DATABASE_URL", "new".into())],
            8904,
        );
        assert!(rendered.contains("DATABASE_URL=new"));
        assert!(rendered.contains("localhost:8904"));
        assert!(rendered.contains("127.0.0.1:8904"));
        assert!(!rendered.contains("localhost:8900"));
    }

    #[test]
    fn atomic_write_uses_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        atomic_write_private(&path, b"SECRET=value\n").unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

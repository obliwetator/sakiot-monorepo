//! Environment and path configuration.
//!
//! Values are resolved in one place so the CLI has a predictable precedence:
//! command-line values are applied by the caller, then exported variables win
//! over the root `.env`, which wins over these defaults.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::{FixtureStartup, Source};

pub const DEFAULT_DEV_ACCOUNT_ID: i64 = 999_999_999;
/// The guild used by the local live-testing workflow unless the caller opts
/// into another guild explicitly with `--guild` or the environment.
pub const DEFAULT_FIXTURE_GUILD_ID: i64 = 362_257_054_829_641_758;
pub const DEFAULT_LOCAL_DATABASE_URL: &str =
    "postgres://postgres:password@localhost:54320/sakiot_rouvas";
pub const DEFAULT_TEST_DATABASE_URL: &str =
    "postgres://postgres:password@localhost:54320/sakiot_test";

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
    pub env_file: PathBuf,
    pub compose_file: PathBuf,
    pub database_url: String,
    pub test_database_url: String,
    pub data_dir: PathBuf,
    pub dev_account_id: i64,
    pub port: u16,
    pub local_url: String,
    pub staging_url: String,
    pub frontend_root: PathBuf,
    pub startup_fixtures: FixtureStartup,
    pub fixture_guild: i64,
    pub ssh: Option<String>,
    pub source: Source,
    pub remote_db: Option<String>,
    pub remote_data: Option<PathBuf>,
    pub remote_env_file: Option<PathBuf>,
    pub remote_binary: Option<PathBuf>,
    pub remote_psql: Option<String>,
    pub remote_hydrate: Option<String>,
    pub staging_db: String,
    pub staging_data: PathBuf,
    pub staging_env_file: PathBuf,
    pub staging_psql: Option<String>,
    pub staging_account_id: Option<i64>,
    pub staging_rsync_path: String,
    pub event_margin: String,
    pub import_manifest: String,
}

#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub startup_fixtures: Option<FixtureStartup>,
}

#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub source: Source,
    pub database: String,
    pub data_dir: PathBuf,
    pub env_file: PathBuf,
    pub binary: PathBuf,
}

impl Config {
    pub fn load(root: impl Into<PathBuf>, overrides: Overrides) -> Result<Self> {
        let root = root.into();
        let env_file = root.join(".env");
        if env_file.is_file() {
            dotenvy::from_path(&env_file)
                .with_context(|| format!("failed to load {}", env_file.display()))?;
        }
        Self::from_lookup(root, env_file, overrides, |key| env::var(key).ok())
    }

    pub fn from_map(
        root: impl Into<PathBuf>,
        values: &BTreeMap<String, String>,
        overrides: Overrides,
    ) -> Result<Self> {
        let root = root.into();
        Self::from_lookup(root.clone(), root.join(".env"), overrides, |key| {
            values.get(key).cloned()
        })
    }

    fn from_lookup<F>(
        root: PathBuf,
        env_file: PathBuf,
        overrides: Overrides,
        get: F,
    ) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let value = |key: &str, default: &str| {
            get(key)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        let path_value = |key: &str, default: PathBuf| {
            let default_value = default.to_string_lossy().to_string();
            resolve_path(&root, PathBuf::from(value(key, &default_value)))
        };

        let database_url = value("DATABASE_URL", DEFAULT_LOCAL_DATABASE_URL);
        let test_database_url = value("SAKIOT_TEST_DATABASE_URL", DEFAULT_TEST_DATABASE_URL);
        let data_dir = path_value("SAKIOT_DATA_DIR", root.join("data"));
        let port = value("PORT", "8900")
            .parse::<u16>()
            .context("PORT must be a valid TCP port")?;
        if port == 0 {
            bail!("PORT must not be zero")
        }

        let startup_fixtures = overrides.startup_fixtures.unwrap_or_else(|| {
            value("SAKIOT_DEV_FETCH", "prompt")
                .parse::<StartupFromEnv>()
                .map(|startup| startup.0)
                .unwrap_or(FixtureStartup::Prompt)
        });
        let source = value("SAKIOT_DEV_SOURCE", "staging")
            .parse::<Source>()
            .map_err(|error| anyhow::anyhow!(error))?;

        let remote_db = optional(&get, "SAKIOT_DEV_REMOTE_DB");
        let remote_data = optional_path(&root, &get, "SAKIOT_DEV_REMOTE_DATA");
        let remote_env_file = optional_path(&root, &get, "SAKIOT_DEV_REMOTE_ENV_FILE");
        let remote_binary = optional_path(&root, &get, "SAKIOT_DEV_REMOTE_WEB_BINARY");

        Ok(Self {
            root: root.clone(),
            env_file,
            compose_file: path_value("SAKIOT_DEV_COMPOSE_FILE", root.join("compose.dev.yml")),
            database_url,
            test_database_url,
            data_dir,
            dev_account_id: parse_id(
                &value("DEV_ACCOUNT_ID", &DEFAULT_DEV_ACCOUNT_ID.to_string()),
                "DEV_ACCOUNT_ID",
            )?,
            port,
            local_url: value("SAKIOT_DEV_LOCAL_URL", &format!("http://localhost:{port}")),
            staging_url: value("SAKIOT_DEV_STAGING_URL", "https://staging.patrykstyla.com"),
            frontend_root: path_value("SAKIOT_DEV_FRONTEND_ROOT", root.join("sakiot-stage")),
            startup_fixtures,
            fixture_guild: parse_id(
                &value(
                    "SAKIOT_DEV_FIXTURE_GUILD",
                    &DEFAULT_FIXTURE_GUILD_ID.to_string(),
                ),
                "SAKIOT_DEV_FIXTURE_GUILD",
            )?,
            ssh: optional(&get, "SAKIOT_DEV_SSH"),
            source,
            remote_db,
            remote_data,
            remote_env_file,
            remote_binary,
            remote_psql: optional(&get, "SAKIOT_DEV_REMOTE_PSQL"),
            remote_hydrate: optional(&get, "SAKIOT_DEV_REMOTE_HYDRATE"),
            staging_db: value("SAKIOT_DEV_STAGING_DB", "sakiot_staging"),
            staging_data: path_value(
                "SAKIOT_DEV_STAGING_DATA",
                PathBuf::from("/var/lib/sakiot-staging/data"),
            ),
            staging_env_file: path_value(
                "SAKIOT_DEV_STAGING_ENV_FILE",
                PathBuf::from("/etc/sakiot/staging.env"),
            ),
            staging_psql: optional(&get, "SAKIOT_DEV_STAGING_PSQL"),
            staging_account_id: optional(&get, "SAKIOT_DEV_STAGING_ACCOUNT_ID")
                .map(|value| parse_id(&value, "SAKIOT_DEV_STAGING_ACCOUNT_ID"))
                .transpose()?,
            staging_rsync_path: value("SAKIOT_DEV_STAGING_RSYNC_PATH", "sudo -n -u sakiot rsync"),
            event_margin: value("SAKIOT_DEV_EVENT_MARGIN", "5 minutes"),
            import_manifest: value(
                "SAKIOT_DEV_STAGING_IMPORT_MANIFEST",
                ".imported-from-production.list",
            ),
        })
    }

    pub fn remote(&self, source: Source) -> RemoteConfig {
        let (database, data_dir, env_file, binary) = match source {
            Source::Production => (
                self.remote_db
                    .clone()
                    .unwrap_or_else(|| "sakiot_rouvas".into()),
                self.remote_data
                    .clone()
                    .unwrap_or_else(|| "/var/lib/sakiot/data".into()),
                self.remote_env_file
                    .clone()
                    .unwrap_or_else(|| "/etc/sakiot/production.env".into()),
                self.remote_binary
                    .clone()
                    .unwrap_or_else(|| "/srv/sakiot/current/web/web_server".into()),
            ),
            Source::Staging => (
                self.remote_db
                    .clone()
                    .unwrap_or_else(|| "sakiot_staging".into()),
                self.remote_data
                    .clone()
                    .unwrap_or_else(|| "/var/lib/sakiot-staging/data".into()),
                self.remote_env_file
                    .clone()
                    .unwrap_or_else(|| "/etc/sakiot/staging.env".into()),
                self.remote_binary
                    .clone()
                    .unwrap_or_else(|| "/srv/sakiot-staging/current/web/web_server".into()),
            ),
        };
        RemoteConfig {
            source,
            database,
            data_dir,
            env_file,
            binary,
        }
    }

    pub fn local_database_is_managed(&self) -> bool {
        self.database_url == DEFAULT_LOCAL_DATABASE_URL
    }
}

#[derive(Debug, Clone, Copy)]
struct StartupFromEnv(FixtureStartup);

impl std::str::FromStr for StartupFromEnv {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "ask" | "prompt" => Ok(Self(FixtureStartup::Prompt)),
            "all" | "full" => Ok(Self(FixtureStartup::Full)),
            "skip" | "none" => Ok(Self(FixtureStartup::Skip)),
            "custom" => Ok(Self(FixtureStartup::Custom)),
            _ => Err(()),
        }
    }
}

fn optional<F>(get: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    get(key).filter(|value| !value.trim().is_empty())
}

fn optional_path<F>(root: &Path, get: &F, key: &str) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    optional(get, key).map(|value| resolve_path(root, PathBuf::from(value)))
}

fn resolve_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn parse_id(value: &str, key: &str) -> Result<i64> {
    let value = value
        .parse::<i64>()
        .with_context(|| format!("{key} must be an unsigned integer"))?;
    if value < 0 {
        bail!("{key} must be an unsigned integer")
    }
    Ok(value)
}

pub fn discover_root(start: impl AsRef<Path>) -> Result<PathBuf> {
    let mut current = start.as_ref().canonicalize().with_context(|| {
        format!(
            "could not resolve working directory {}",
            start.as_ref().display()
        )
    })?;
    if current.is_file() {
        current.pop();
    }
    loop {
        if current.join("Cargo.toml").is_file() && current.join("compose.dev.yml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            bail!(
                "could not find the Sakiot repository root from {}",
                start.as_ref().display()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_overrides_environment_and_defaults() {
        let root = PathBuf::from("/tmp/sakiot-dev-config");
        let mut values = BTreeMap::new();
        values.insert("PORT".into(), "9010".into());
        values.insert("SAKIOT_DATA_DIR".into(), "media".into());
        values.insert("SAKIOT_DEV_FETCH".into(), "all".into());

        let config = Config::from_map(
            &root,
            &values,
            Overrides {
                startup_fixtures: Some(FixtureStartup::Skip),
            },
        )
        .unwrap();
        assert_eq!(config.port, 9010);
        assert_eq!(config.data_dir, root.join("media"));
        assert_eq!(config.startup_fixtures, FixtureStartup::Skip);
        assert_eq!(config.fixture_guild, DEFAULT_FIXTURE_GUILD_ID);
    }

    #[test]
    fn fixture_guild_can_be_overridden() {
        let root = PathBuf::from("/tmp/sakiot-dev-config");
        let mut values = BTreeMap::new();
        values.insert("SAKIOT_DEV_FIXTURE_GUILD".into(), "42".into());

        let config = Config::from_map(&root, &values, Overrides::default()).unwrap();

        assert_eq!(config.fixture_guild, 42);
    }

    #[test]
    fn source_legacy_aliases_are_accepted() {
        assert_eq!("prod".parse::<Source>().unwrap(), Source::Production);
        assert_eq!("production".parse::<Source>().unwrap(), Source::Production);
    }
}

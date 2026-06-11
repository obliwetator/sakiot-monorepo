//! Deploy mode and runtime configuration, ported from the argument and
//! environment handling at the top of ops/deploy-release.sh.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Release,
    Rollback,
    Stage,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Release => "release",
            Mode::Rollback => "rollback",
            Mode::Stage => "stage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Production,
    Staging,
}

impl Target {
    pub fn as_str(self) -> &'static str {
        match self {
            Target::Production => "production",
            Target::Staging => "staging",
        }
    }
}

/// Parsed positional arguments: mode, tag, sha, schema override.
#[derive(Debug, Clone)]
pub struct Request {
    pub mode: Mode,
    pub target: Target,
    pub tag: String,
    pub sha: String,
    /// Raw fourth argument; anything other than --allow-schema-mismatch is
    /// rejected by validate() with the bash error message.
    pub schema_option: Option<String>,
    pub dry_run: bool,
}

impl Request {
    pub fn allow_schema_mismatch(&self) -> bool {
        self.schema_option.as_deref() == Some("--allow-schema-mismatch")
    }
}

impl Request {
    /// Mirrors deploy-release.sh lines 11-34 plus the ops/deploy wrapper
    /// arity checks. `args` excludes argv[0]. A leading `--dry-run` is a
    /// local affordance not reachable through the SSH forced command.
    pub fn parse(mut args: Vec<String>) -> Result<Request, UsageError> {
        let dry_run = args.first().is_some_and(|a| a == "--dry-run");
        if dry_run {
            args.remove(0);
        }

        let mode = args.first().map(String::as_str);
        match mode {
            Some("release") | Some("rollback") => {
                let is_rollback = mode == Some("rollback");
                let (min, max) = if is_rollback { (3, 4) } else { (3, 3) };
                if args.len() < min || args.len() > max {
                    return Err(UsageError(if is_rollback {
                        "usage: sakiot-deploy rollback <tag> <sha> [--allow-schema-mismatch]"
                    } else {
                        "usage: sakiot-deploy release <tag> <sha>"
                    }));
                }
                Ok(Request {
                    mode: if is_rollback {
                        Mode::Rollback
                    } else {
                        Mode::Release
                    },
                    target: Target::Production,
                    tag: args[1].clone(),
                    sha: args[2].clone(),
                    schema_option: args.get(3).cloned().filter(|option| !option.is_empty()),
                    dry_run,
                })
            }
            Some("stage") => {
                if args.len() != 2 {
                    return Err(UsageError("usage: sakiot-deploy stage <sha>"));
                }
                Ok(Request {
                    mode: Mode::Stage,
                    target: Target::Staging,
                    tag: "main".to_string(),
                    sha: args[1].clone(),
                    schema_option: None,
                    dry_run,
                })
            }
            _ => Err(UsageError(
                "usage: sakiot-deploy [--dry-run] {release|rollback|stage} ...",
            )),
        }
    }

    /// Validates tag/sha/option shapes; mirrors deploy-release.sh lines 19-23.
    pub fn validate(&self) -> Result<()> {
        if self.target == Target::Production {
            crate::validate::validate_tag(&self.tag)?;
        }
        crate::validate::validate_sha(&self.sha)?;
        if let Some(option) = &self.schema_option
            && option != "--allow-schema-mismatch"
        {
            bail!("invalid rollback option");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct UsageError(pub &'static str);

/// Everything deploy-release.sh reads from the environment (lines 36-64),
/// resolved with the same defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub env_file: PathBuf,
    pub web_unit: String,
    pub bot_unit_prefix: String,
    pub repository_url: String,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub release_root: PathBuf,
    pub current_root: PathBuf,
    pub cache_dir: PathBuf,
    pub frontend_root: PathBuf,
    pub web_health_url: String,
    pub web_registry_url: String,
    pub legacy_bot_unit: String,
    pub legacy_bot_grpc: String,
    pub legacy_web_enabled: bool,
    pub systemctl_use_sudo: bool,
    pub skip_db_backup: bool,
    pub rollback_force_rebuild: bool,
    pub keep_releases: String,
    pub registry_secret: String,
    pub database_url: Option<String>,
    pub test_database_url: String,
}

impl Config {
    pub fn source_repo(&self) -> PathBuf {
        self.cache_dir.join("repository")
    }

    pub fn worktree_root(&self) -> PathBuf {
        self.cache_dir.join("worktrees")
    }

    pub fn default_env_file(target: Target) -> &'static str {
        match target {
            Target::Production => "/etc/sakiot/production.env",
            Target::Staging => "/etc/sakiot/staging.env",
        }
    }

    /// Loads the env file (if present) into the process environment with
    /// override semantics, matching `set -a; source ...; set +a`, then reads
    /// configuration. The strict-parse guard rejects shell-flavored lines
    /// dotenvy would silently misread.
    pub fn load(target: Target) -> Result<Config> {
        let env_file = std::env::var("SAKIOT_ENV_FILE")
            .unwrap_or_else(|_| Config::default_env_file(target).to_string());
        let env_file = PathBuf::from(env_file);
        if env_file.is_file() {
            check_env_file_is_plain(&env_file)?;
            dotenvy::from_path_override(&env_file)
                .with_context(|| format!("failed to load env file {}", env_file.display()))?;
        }
        Config::from_env(env_file)
    }

    pub fn from_env(env_file: PathBuf) -> Result<Config> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

        let repository_url = var("SAKIOT_REPOSITORY_URL").context("set SAKIOT_REPOSITORY_URL")?;
        if !repository_url.starts_with("https://") {
            bail!("SAKIOT_REPOSITORY_URL must use HTTPS");
        }

        let flag = |name: &str, default: &str| {
            std::env::var(name).unwrap_or_else(|_| default.to_string()) == "1"
        };

        Ok(Config {
            env_file,
            web_unit: var("SAKIOT_WEB_UNIT").unwrap_or_else(|| "sakiot-web.service".into()),
            bot_unit_prefix: var("SAKIOT_BOT_UNIT_PREFIX")
                .unwrap_or_else(|| "sakiot-fbi-agent@".into()),
            repository_url,
            data_dir: var("SAKIOT_DATA_DIR")
                .unwrap_or_else(|| "/var/lib/sakiot/data".into())
                .into(),
            state_dir: var("SAKIOT_DEPLOY_STATE_DIR")
                .unwrap_or_else(|| "/var/lib/sakiot/deploy".into())
                .into(),
            release_root: var("SAKIOT_RELEASE_ROOT")
                .unwrap_or_else(|| "/srv/sakiot/releases".into())
                .into(),
            current_root: var("SAKIOT_CURRENT_ROOT")
                .unwrap_or_else(|| "/srv/sakiot/current".into())
                .into(),
            cache_dir: var("SAKIOT_CACHE_DIR")
                .unwrap_or_else(|| "/var/cache/sakiot".into())
                .into(),
            frontend_root: var("SAKIOT_FRONTEND_ROOT")
                .unwrap_or_else(|| "/var/www/patrykstyla.com".into())
                .into(),
            web_health_url: var("SAKIOT_WEB_HEALTH_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8900/healthz".into()),
            web_registry_url: var("SAKIOT_WEB_REGISTRY_URL").unwrap_or_else(|| {
                "http://127.0.0.1:8900/internal/fbi-agent/grpc-endpoints".into()
            }),
            legacy_bot_unit: var("SAKIOT_LEGACY_BOT_UNIT").unwrap_or_default(),
            legacy_bot_grpc: var("SAKIOT_LEGACY_BOT_GRPC").unwrap_or_default(),
            legacy_web_enabled: flag("SAKIOT_LEGACY_WEB_ENABLED", "0"),
            systemctl_use_sudo: flag("SAKIOT_SYSTEMCTL_USE_SUDO", "1"),
            skip_db_backup: flag("SAKIOT_SKIP_DB_BACKUP", "0"),
            rollback_force_rebuild: flag("SAKIOT_ROLLBACK_FORCE_REBUILD", "0"),
            keep_releases: std::env::var("SAKIOT_KEEP_RELEASES").unwrap_or_else(|_| "5".into()),
            registry_secret: var("FBI_AGENT_REGISTRY_SECRET").unwrap_or_default(),
            database_url: var("DATABASE_URL"),
            test_database_url: var("SAKIOT_TEST_DATABASE_URL").unwrap_or_default(),
        })
    }
}

/// The env files are plain KEY=value (see ops/*.env.example). Bash `source`
/// would expand anything shell-flavored; dotenvy would not. Refuse to guess.
fn check_env_file_is_plain(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read env file {}", path.display()))?;
    for (number, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let valid_key = assignment.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty()
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !key.as_bytes()[0].is_ascii_digit()
        });
        let shell_flavored = assignment.contains('$') || assignment.contains('`');
        if !valid_key || shell_flavored {
            bail!(
                "env file {} line {} is not plain KEY=value; refusing to load it",
                path.display(),
                number + 1
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Request, UsageError> {
        Request::parse(args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn parses_all_modes() {
        let release = parse(&["release", "v1.2.3", &"a".repeat(40)]).unwrap();
        assert_eq!(release.mode, Mode::Release);
        assert_eq!(release.target, Target::Production);

        let rollback = parse(&[
            "rollback",
            "v1.2.3",
            &"a".repeat(40),
            "--allow-schema-mismatch",
        ])
        .unwrap();
        assert_eq!(rollback.mode, Mode::Rollback);
        assert!(rollback.allow_schema_mismatch());

        let stage = parse(&["stage", &"a".repeat(40)]).unwrap();
        assert_eq!(stage.mode, Mode::Stage);
        assert_eq!(stage.tag, "main");
    }

    #[test]
    fn rejects_bad_arity_and_modes() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["deploy"]).is_err());
        assert!(parse(&["release", "v1.2.3"]).is_err());
        assert!(parse(&["release", "v1.2.3", "sha", "extra"]).is_err());
        assert!(parse(&["stage"]).is_err());
        assert!(parse(&["stage", "sha", "extra"]).is_err());
    }

    #[test]
    fn dry_run_flag_strips() {
        let request = parse(&["--dry-run", "stage", &"a".repeat(40)]).unwrap();
        assert!(request.dry_run);
    }

    #[test]
    fn invalid_rollback_option_rejected() {
        let request = parse(&["rollback", "v1.2.3", &"a".repeat(40), "--force"]).unwrap();
        let error = request.validate().unwrap_err().to_string();
        assert_eq!(error, "invalid rollback option");
    }

    #[test]
    fn env_file_guard_accepts_plain_and_rejects_shell() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.env");
        std::fs::write(
            &plain,
            "# comment\n\nSAKIOT_DATA_DIR=/var/lib/sakiot/data\nexport FOO=bar\nURL=https://x/y?a=b\n",
        )
        .unwrap();
        assert!(check_env_file_is_plain(&plain).is_ok());

        for bad in [
            "FOO=$(whoami)\n",
            "FOO=${HOME}/x\n",
            "FOO=`id`\n",
            "1BAD=value\n",
            "not an assignment\n",
        ] {
            let file = dir.path().join("bad.env");
            std::fs::write(&file, bad).unwrap();
            assert!(check_env_file_is_plain(&file).is_err(), "{bad:?}");
        }
    }
}

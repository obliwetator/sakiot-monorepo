//! The deploy orchestrator: a line-for-line port of ops/deploy-release.sh
//! lines 36-601. Phase order, log lines, error messages, state files, and
//! the manifest stay byte-compatible with the bash engine so releases made
//! by either engine remain valid rollback targets for the other.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::admin_api::AdminApi;
use crate::clock::Clock;
use crate::components::{Component, all_components, component_selected, components_for_paths};
use crate::config::{Config, Mode, Request, Target};
use crate::fsx;
use crate::git;
use crate::lock::DeployLock;
use crate::log;
use crate::release::{
    Manifest, ManifestDatabase, ManifestReused, prune_old_releases, release_id, reusable_artifact,
};
use crate::runner::{Cmd, CommandRunner};
use crate::systemctl::Systemctl;
use crate::validate;
use crate::web_api::WebApi;

/// How long recovery waits for a failed new bot to stop before SIGKILL.
/// A bot without sessions stops in seconds; anything longer is the
/// SIGTERM-hang failure mode, and bot units never time out on their own.
const RECOVERY_STOP_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Deps<'a> {
    pub runner: &'a dyn CommandRunner,
    pub admin: &'a dyn AdminApi,
    pub web: &'a dyn WebApi,
    pub clock: &'a dyn Clock,
    pub hostname: String,
    pub free_port: &'a dyn Fn() -> Result<u16>,
    /// `command -v` parity; injectable so tests don't depend on host PATH.
    pub require_command: &'a dyn Fn(&str) -> Result<()>,
}

/// `command -v` parity: the deploy fails up front when a required tool is
/// missing rather than midway through a release.
pub fn require_command(name: &str) -> Result<()> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file()
            && std::fs::metadata(&candidate)
                .map(|meta| meta.mode() & 0o111 != 0)
                .unwrap_or(false)
    });
    if !found {
        bail!("required command not found: {name}");
    }
    Ok(())
}

/// Bot blue/green handoff state, shared with the recovery path. Mirrors the
/// variables read by recover_bot_on_error() in deploy-release.sh.
#[derive(Default)]
struct BotHandoff {
    recovery_required: bool,
    new_bot_started: bool,
    handoff_pending: bool,
    old_bot_disabled: bool,
    new_bot_unit: String,
    new_bot_grpc: String,
    old_bot_unit: String,
    old_bot_grpc: String,
    old_bot_is_legacy: bool,
    previous_bot_unit: String,
    previous_bot_grpc: String,
}

impl BotHandoff {
    /// cancel_old_bot_drain() from deploy-release.sh.
    fn cancel_old_drain(&self, deps: &Deps, systemctl: &Systemctl, release_id: &str) {
        if !self.recovery_required || self.old_bot_grpc.is_empty() {
            return;
        }
        log("new FBI Agent failed readiness; cancelling old instance drain");
        let reason = format!("release {release_id} failed readiness");
        if deps.admin.cancel_drain(&self.old_bot_grpc, &reason).is_ok() {
            return;
        }
        if self.old_bot_is_legacy {
            log("legacy FBI Agent lacks CancelDrain; restarting it to clear drain state");
            if !systemctl.run_ok(&["legacy-bot-restart", &self.old_bot_unit]) {
                log("legacy FBI Agent restart unavailable; manual restart required");
            }
        }
    }

    /// recover_bot_on_error() from deploy-release.sh. Best-effort: every step
    /// runs even if earlier ones fail.
    fn recover(
        &self,
        deps: &Deps,
        systemctl: &Systemctl,
        state_dir: &Path,
        release_id: &str,
        registry_url: &str,
        registry_secret: &str,
    ) {
        if self.new_bot_started && !self.new_bot_unit.is_empty() {
            systemctl.stop_bot_bounded(&self.new_bot_unit, RECOVERY_STOP_TIMEOUT);
            let _ = systemctl.run_ok(&["disable", &self.new_bot_unit]);
            let _ = fsx::write_line(&state_dir.join("current-bot.unit"), &self.previous_bot_unit);
            let _ = fsx::write_line(&state_dir.join("current-bot.grpc"), &self.previous_bot_grpc);
        }
        if self.old_bot_disabled && !self.old_bot_unit.is_empty() {
            if self.old_bot_is_legacy {
                let _ = systemctl.run_ok(&["legacy-bot-enable", &self.old_bot_unit]);
            } else {
                let _ = systemctl.run_ok(&["enable", &self.old_bot_unit]);
            }
        }
        self.cancel_old_drain(deps, systemctl, release_id);
        if !self.old_bot_grpc.is_empty()
            && deps
                .web
                .publish_registry(registry_url, registry_secret, &self.old_bot_grpc, &[])
                .is_err()
        {
            log("failed to restore old FBI Agent in web registry");
        }
    }
}

pub fn run(request: &Request, config: &Config, deps: &Deps) -> Result<()> {
    request.validate()?;
    let mode = request.mode;
    let target = request.target;
    let tag = &request.tag;
    let sha = &request.sha;

    // deploy-release.sh lines 66-75: tool availability. flock/jq/curl/python3
    // and grpcurl are no longer shelled out to; their work happens in-process.
    for command in ["git", "cargo", "rsync"] {
        (deps.require_command)(command)?;
    }
    let systemctl = Systemctl::new(deps.runner, config.systemctl_use_sudo);
    if config.systemctl_use_sudo {
        (deps.require_command)("sudo")?;
        if !is_executable(Path::new(crate::systemctl::WRAPPER_PATH)) {
            bail!("systemctl wrapper is not installed");
        }
    } else {
        (deps.require_command)("systemctl")?;
    }

    // Lines 77-81: directory layout.
    for dir in [
        &config.state_dir,
        &config.state_dir.join("tags"),
        &config.release_root,
        &config.current_root,
        &config.cache_dir,
        &config.worktree_root(),
    ] {
        fsx::ensure_dir_mode(dir, 0o750)?;
    }
    for dir in [
        config.data_dir.clone(),
        config.data_dir.join("voice_recordings"),
        config.data_dir.join("no_silence_voice_recordings"),
        config.data_dir.join("waveform_data"),
        config.data_dir.join("clips"),
    ] {
        fsx::ensure_dir_mode(&dir, 0o755)?;
    }

    // Lines 83-84: one deploy at a time; shared with the bash engine.
    let _lock = DeployLock::acquire(&config.state_dir.join("deploy.lock"))?;

    // Lines 86-110: repository cache, tag verification, tag record.
    let source_repo = config.source_repo();
    git::ensure_cache(deps.runner, &source_repo, &config.repository_url)?;
    if target == Target::Production {
        git::fetch_production(deps.runner, &source_repo, tag)?;
        let resolved_sha = git::resolve_tag(deps.runner, &source_repo, tag)?;
        if &resolved_sha != sha {
            bail!("tag {tag} resolves to {resolved_sha}, not supplied SHA {sha}");
        }
    } else {
        git::fetch_staging(deps.runner, &source_repo)?;
    }
    git::require_commit(deps.runner, &source_repo, sha)?;

    let tag_record = config.state_dir.join("tags").join(tag);
    if target == Target::Production {
        validate::validate_tag_record(mode, &tag_record, tag, sha)?;
    }

    let previous_sha = fsx::read_line(&config.state_dir.join("current.sha")).unwrap_or_default();
    let previous_tag = fsx::read_line(&config.state_dir.join("current.tag")).unwrap_or_default();

    // Lines 115-122: refuse rollbacks that cross schema changes.
    if mode == Mode::Rollback && !previous_sha.is_empty() && !request.allow_schema_mismatch() {
        let migration_changes = git::diff_names(
            deps.runner,
            &source_repo,
            sha,
            &previous_sha,
            Some("sakiot-db/migrations"),
        )?;
        if !migration_changes.is_empty() {
            bail!("rollback crosses migration changes; use explicit schema compatibility override");
        }
    }

    // Lines 124-133: release identity.
    let timestamp = crate::clock::compact_timestamp(deps.clock.now_utc())?;
    let release_id = release_id(mode, tag, sha, &timestamp);
    let artifact_dir = config.release_root.join(&release_id);
    let worktree_path = config.worktree_root().join(&release_id);

    // Lines 142-152: component selection.
    let mut changed_paths: Option<Vec<String>> = None;
    let components: Vec<Component> = if mode == Mode::Rollback {
        vec![Component::Bot, Component::Web, Component::Frontend]
    } else if previous_sha.is_empty() {
        all_components()
    } else {
        let paths = git::diff_names(deps.runner, &source_repo, &previous_sha, sha, None)?;
        let components = components_for_paths(&paths);
        changed_paths = Some(paths);
        components
    };
    if components.is_empty() {
        log("documentation-only release; no application components selected");
    }

    if artifact_dir.exists() {
        bail!(
            "release directory already exists: {}",
            artifact_dir.display()
        );
    }

    // Lines 163-175: artifact reuse on rollback.
    let mut reuse_bot: Option<PathBuf> = None;
    let mut reuse_web: Option<PathBuf> = None;
    let mut reuse_frontend: Option<PathBuf> = None;
    if mode == Mode::Rollback && !config.rollback_force_rebuild {
        if component_selected(Component::Bot, &components) {
            reuse_bot =
                reusable_artifact(&config.release_root, sha, Component::Bot, &artifact_dir)?;
        }
        if component_selected(Component::Web, &components) {
            reuse_web =
                reusable_artifact(&config.release_root, sha, Component::Web, &artifact_dir)?;
        }
        if component_selected(Component::Frontend, &components) {
            reuse_frontend = reusable_artifact(
                &config.release_root,
                sha,
                Component::Frontend,
                &artifact_dir,
            )?;
        }
    }

    let build_bot = component_selected(Component::Bot, &components) && reuse_bot.is_none();
    let build_web = component_selected(Component::Web, &components) && reuse_web.is_none();
    let build_rust = build_bot || build_web;

    if request.dry_run {
        log(format!(
            "dry run: {} {} -> release {release_id}",
            mode.as_str(),
            sha
        ));
        log(format!(
            "dry run: components: {}",
            components
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ));
        log(format!(
            "dry run: reuse bot={} web={} frontend={}; rust build needed: {build_rust}",
            reuse_bot.is_some(),
            reuse_web.is_some(),
            reuse_frontend.is_some()
        ));
        log("dry run: stopping before artifact build, migrations, and service handoff");
        return Ok(());
    }

    fsx::ensure_dir_mode(&artifact_dir, 0o755)?;

    // Lines 135-140: detached worktree at the release SHA, cleaned on exit.
    let worktree = git::Worktree::add(deps.runner, &source_repo, &worktree_path, sha)?;

    // Lines 177-196: test the Rust workspace when building from source.
    let cargo_target = config.cache_dir.join("cargo-target");
    if build_rust {
        (deps.require_command)("protoc")?;
        let database_url = config.database_url.clone().context("set DATABASE_URL")?;
        validate::validate_test_database_url(&database_url, &config.test_database_url)?;
        log("testing Rust workspace");
        let test_data_dir = tempfile::Builder::new()
            .prefix("test-data.")
            .tempdir_in(&config.cache_dir)
            .context("failed to create test data directory")?;
        deps.runner.run(
            &Cmd::new("cargo")
                .args(["test", "--workspace", "--locked"])
                .cwd(worktree.path())
                .env("DATABASE_URL", &config.test_database_url)
                .env(
                    "SAKIOT_DATA_DIR",
                    test_data_dir.path().display().to_string(),
                )
                .env("SQLX_OFFLINE", "true")
                .env("CARGO_TARGET_DIR", cargo_target.display().to_string()),
        )?;
    }

    // Lines 198-232: bot and web binaries (build or reuse).
    if component_selected(Component::Bot, &components) {
        fsx::ensure_dir_mode(&artifact_dir.join("fbi-agent"), 0o755)?;
        if let Some(reuse) = &reuse_bot {
            log(format!(
                "reusing FBI Agent artifact from {}",
                reuse.display()
            ));
            fsx::install_file(
                &reuse.join("fbi-agent/fbi_agent"),
                &artifact_dir.join("fbi-agent/fbi_agent"),
                0o755,
            )?;
        } else {
            log("building FBI Agent");
            deps.runner.run(
                &Cmd::new("cargo")
                    .args(["build", "--release", "--locked", "--package", "fbi_agent"])
                    .cwd(worktree.path())
                    .env("SQLX_OFFLINE", "true")
                    .env("CARGO_TARGET_DIR", cargo_target.display().to_string()),
            )?;
            fsx::install_file(
                &cargo_target.join("release/fbi_agent"),
                &artifact_dir.join("fbi-agent/fbi_agent"),
                0o755,
            )?;
        }
    }

    if component_selected(Component::Web, &components) {
        fsx::ensure_dir_mode(&artifact_dir.join("web"), 0o755)?;
        if let Some(reuse) = &reuse_web {
            log(format!(
                "reusing web server artifact from {}",
                reuse.display()
            ));
            fsx::install_file(
                &reuse.join("web/web_server"),
                &artifact_dir.join("web/web_server"),
                0o755,
            )?;
        } else {
            log("building web server");
            deps.runner.run(
                &Cmd::new("cargo")
                    .args(["build", "--release", "--locked", "--package", "web_server"])
                    .cwd(worktree.path())
                    .env("SQLX_OFFLINE", "true")
                    .env("CARGO_TARGET_DIR", cargo_target.display().to_string()),
            )?;
            fsx::install_file(
                &cargo_target.join("release/web_server"),
                &artifact_dir.join("web/web_server"),
                0o755,
            )?;
        }
    }

    // Lines 234-253: frontend bundle (build or reuse).
    if component_selected(Component::Frontend, &components) {
        fsx::ensure_dir_mode(&artifact_dir.join("frontend"), 0o755)?;
        if let Some(reuse) = &reuse_frontend {
            log(format!(
                "reusing frontend artifact from {}",
                reuse.display()
            ));
            deps.runner.run(&Cmd::new("cp").arg("-a").args([
                reuse.join("frontend/dist").display().to_string(),
                artifact_dir.join("frontend/dist").display().to_string(),
            ]))?;
        } else {
            (deps.require_command)("bun")?;
            log("testing and building frontend");
            let stage_dir = worktree.path().join("sakiot_stage");
            deps.runner.run(
                &Cmd::new("bun")
                    .args(["install", "--frozen-lockfile"])
                    .cwd(&stage_dir),
            )?;
            deps.runner
                .run(&Cmd::new("bun").args(["run", "test"]).cwd(&stage_dir))?;
            deps.runner.run(
                &Cmd::new("bun")
                    .args(["run", "build"])
                    .cwd(&stage_dir)
                    .env("SAKIOT_RELEASE_TAG", tag)
                    .env("SAKIOT_COMMIT_SHA", sha)
                    .env("SAKIOT_BUNDLE_VERSION", &release_id),
            )?;
            deps.runner.run(&Cmd::new("cp").arg("-a").args([
                stage_dir.join("dist").display().to_string(),
                artifact_dir.join("frontend/dist").display().to_string(),
            ]))?;
        }
    }

    // The bash engine ran ops/tests/run.sh from the worktree here. The engine
    // is no longer shipped in the release tag: its tests run in CI and in
    // `cargo test --workspace` above, and the remaining bash suites cover the
    // out-of-band installed shims, so deploy-time execution validated code
    // that is not what executes.

    // Lines 260-280: migrations, with pre-migrate backup on production.
    let migration_head = git::migration_head(&worktree.path().join("sakiot-db/migrations"))?;
    let mut migrations_ran = false;
    if component_selected(Component::Database, &components) {
        (deps.require_command)("sqlx")?;
        log("checking migration state");
        let migrations_source = worktree.path().join("sakiot-db/migrations");
        deps.runner.run(&Cmd::new("sqlx").args([
            "migrate",
            "info",
            "--source",
            &migrations_source.display().to_string(),
        ]))?;
        if config.skip_db_backup {
            log("SAKIOT_SKIP_DB_BACKUP=1: applying migrations without a pre-migrate backup");
            deps.runner.run(&Cmd::new("sqlx").args([
                "migrate",
                "run",
                "--source",
                &migrations_source.display().to_string(),
            ]))?;
        } else {
            (deps.require_command)("pg_dump")?;
            (deps.require_command)("age")?;
            log("backing up database and applying pending migrations");
            deps.runner.run(
                &Cmd::new(
                    worktree
                        .path()
                        .join("sakiot-db/ops/backup/pre-migrate-backup.sh")
                        .display()
                        .to_string(),
                )
                .env("SAKIOT_ENV_FILE", config.env_file.display().to_string()),
            )?;
        }
        migrations_ran = true;
    }

    // Lines 282-539: service handoff under the bot recovery scope.
    let mut bot = BotHandoff::default();
    let handoff = deploy_services(
        config,
        deps,
        &systemctl,
        &components,
        &artifact_dir,
        worktree.path(),
        &release_id,
        &mut bot,
    );
    if let Err(error) = handoff {
        bot.recover(
            deps,
            &systemctl,
            &config.state_dir,
            &release_id,
            &config.web_registry_url,
            &config.registry_secret,
        );
        return Err(error);
    }

    // Lines 541-594: manifest and state recording.
    let manifest_path = artifact_dir.join("manifest.json");
    let manifest = Manifest {
        target: target.as_str().to_string(),
        mode: mode.as_str().to_string(),
        tag: tag.clone(),
        sha: sha.clone(),
        previous_tag,
        previous_sha,
        release_id: release_id.clone(),
        components: components.iter().map(|c| c.as_str().to_string()).collect(),
        changed_paths: changed_paths.unwrap_or_default(),
        database: ManifestDatabase {
            migrations_ran,
            migration_head,
        },
        reused: ManifestReused {
            bot: reuse_bot.is_some(),
            web: reuse_web.is_some(),
            frontend: reuse_frontend.is_some(),
        },
        deployed_at: crate::clock::rfc3339_timestamp(deps.clock.now_utc())?,
    };
    manifest.write(&manifest_path)?;

    fsx::write_line_atomic(&config.state_dir.join("current.sha"), sha)?;
    fsx::write_line_atomic(&config.state_dir.join("current.tag"), tag)?;
    fsx::write_line_atomic(
        &config.state_dir.join("current.manifest"),
        &manifest_path.display().to_string(),
    )?;
    if mode == Mode::Release {
        fsx::write_line(&tag_record, sha)?;
    }

    // Lines 596-601: garbage collection and the final summary.
    if let Err(error) = prune_old_releases(
        &systemctl,
        &config.release_root,
        &config.current_root,
        &config.state_dir,
        &config.keep_releases,
        &config.bot_unit_prefix,
    ) {
        log(format!(
            "release pruning encountered an error; continuing ({error})"
        ));
    }

    log(format!("{} complete: {release_id}", mode.as_str()));
    log(format!(
        "newest {} releases retained for rollback",
        config.keep_releases
    ));
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Bot blue/green handoff, web swap, frontend publish, and handoff
/// completion (deploy-release.sh lines 360-539). Any error here unwinds
/// through BotHandoff::recover, mirroring the bash ERR trap.
#[allow(clippy::too_many_arguments)]
fn deploy_services(
    config: &Config,
    deps: &Deps,
    systemctl: &Systemctl,
    components: &[Component],
    artifact_dir: &Path,
    worktree_path: &Path,
    release_id: &str,
    bot: &mut BotHandoff,
) -> Result<()> {
    if component_selected(Component::Bot, components) {
        bot.old_bot_unit =
            fsx::read_line(&config.state_dir.join("current-bot.unit")).unwrap_or_default();
        bot.old_bot_grpc =
            fsx::read_line(&config.state_dir.join("current-bot.grpc")).unwrap_or_default();
        bot.previous_bot_unit = bot.old_bot_unit.clone();
        bot.previous_bot_grpc = bot.old_bot_grpc.clone();
        bot.new_bot_unit = format!("{}{release_id}.service", config.bot_unit_prefix);
        bot.new_bot_grpc = format!("127.0.0.1:{}", (deps.free_port)()?);

        fsx::write_line(
            &artifact_dir.join("fbi-agent/service.env"),
            &format!(
                "BOT_ROLE=active\nBOT_INSTANCE_ID={hostname}-{release_id}\nRELEASE_ID={release_id}\nGRPC_ADDR={grpc}\nDRAIN_TIMEOUT_SECONDS=0\nSAKIOT_DATA_DIR={data_dir}",
                hostname = deps.hostname,
                grpc = bot.new_bot_grpc,
                data_dir = config.data_dir.display(),
            ),
        )?;
        std::fs::set_permissions(
            artifact_dir.join("fbi-agent/service.env"),
            std::fs::Permissions::from_mode(0o640),
        )?;

        let mut old_bot_active = false;
        if !bot.old_bot_unit.is_empty()
            && systemctl.run_ok(&["is-active", "--quiet", &bot.old_bot_unit])
        {
            old_bot_active = true;
        } else if bot.old_bot_unit.is_empty()
            && !config.legacy_bot_unit.is_empty()
            && !config.legacy_bot_grpc.is_empty()
            && systemctl.run_ok(&["legacy-bot-is-active", &config.legacy_bot_unit])
        {
            bot.old_bot_unit = config.legacy_bot_unit.clone();
            bot.old_bot_grpc = config.legacy_bot_grpc.clone();
            bot.old_bot_is_legacy = true;
            old_bot_active = true;
            log(format!(
                "adopting legacy FBI Agent {} for first-release handoff",
                bot.old_bot_unit
            ));
        }

        if old_bot_active {
            if bot.old_bot_grpc.is_empty() {
                bail!("current FBI Agent is missing its gRPC address");
            }
            log(format!("draining {}", bot.old_bot_unit));
            deps.admin
                .start_drain(&bot.old_bot_grpc, &format!("deploy {release_id}"))?;
            bot.recovery_required = true;
        } else {
            bot.old_bot_unit = String::new();
            bot.old_bot_grpc = String::new();
        }

        log(format!("starting {}", bot.new_bot_unit));
        if !systemctl.run_ok(&["start", &bot.new_bot_unit]) {
            // Bash exits 1 here without the full ERR-trap recovery: stop the
            // unit, cancel the old drain, and leave registry/state untouched.
            systemctl.stop_bot_bounded(&bot.new_bot_unit, RECOVERY_STOP_TIMEOUT);
            bot.cancel_old_drain(deps, systemctl, release_id);
            *bot = BotHandoff::default();
            bail!("failed to start new FBI Agent unit");
        }
        bot.new_bot_started = true;

        let mut bot_ready = false;
        for _ in 0..30 {
            if deps.admin.drain_status_ok(&bot.new_bot_grpc) {
                bot_ready = true;
                break;
            }
            deps.clock.sleep(Duration::from_secs(1));
        }
        if !bot_ready {
            // Same shape as the start failure: bash `die`s here, bypassing
            // recover_bot_on_error (state files and registry are untouched).
            systemctl.stop_bot_bounded(&bot.new_bot_unit, RECOVERY_STOP_TIMEOUT);
            bot.new_bot_started = false;
            bot.cancel_old_drain(deps, systemctl, release_id);
            *bot = BotHandoff::default();
            bail!("new FBI Agent failed readiness");
        }

        systemctl.run(&["enable", &bot.new_bot_unit])?;
        fsx::write_line(
            &config.state_dir.join("current-bot.unit"),
            &bot.new_bot_unit,
        )?;
        fsx::write_line(
            &config.state_dir.join("current-bot.grpc"),
            &bot.new_bot_grpc,
        )?;

        let draining: Vec<String> = if bot.old_bot_grpc.is_empty() {
            Vec::new()
        } else {
            vec![bot.old_bot_grpc.clone()]
        };
        if deps
            .web
            .publish_registry(
                &config.web_registry_url,
                &config.registry_secret,
                &bot.new_bot_grpc,
                &draining,
            )
            .is_err()
        {
            log("web server registry unavailable; web release env will use new endpoint");
        }

        if !bot.old_bot_grpc.is_empty() {
            bot.handoff_pending = true;
        }
    }

    if component_selected(Component::Web, components) {
        let active_bot_grpc =
            fsx::read_line(&config.state_dir.join("current-bot.grpc")).unwrap_or_default();
        let grpc_address = if active_bot_grpc.is_empty() {
            "127.0.0.1:50052".to_string()
        } else {
            active_bot_grpc
        };
        fsx::write_line(
            &artifact_dir.join("web/service.env"),
            &format!(
                "RELEASE_ID={release_id}\nSAKIOT_DATA_DIR={}\nGRPC_ADDRESS=http://{grpc_address}",
                config.data_dir.display(),
            ),
        )?;
        std::fs::set_permissions(
            artifact_dir.join("web/service.env"),
            std::fs::Permissions::from_mode(0o640),
        )?;

        let web_link = config.current_root.join("web");
        let previous_web_target = std::fs::read_link(&web_link).ok();
        let mut legacy_web_stopped = false;
        let restore_previous_web = |systemctl: &Systemctl, legacy_web_stopped: bool| {
            if let Some(previous) = &previous_web_target {
                let _ = fsx::atomic_symlink(previous, &web_link);
                let _ = systemctl.run_ok(&["restart", &config.web_unit]);
            } else if legacy_web_stopped {
                let _ = systemctl.run_ok(&["legacy-web-start-enable"]);
            }
        };

        fsx::atomic_symlink(&artifact_dir.join("web"), &web_link)?;
        if previous_web_target.is_none()
            && config.legacy_web_enabled
            && systemctl.run_ok(&["legacy-web-is-active"])
        {
            log("stopping legacy web server for first-release handoff");
            if !systemctl.run_ok(&["legacy-web-stop-disable"]) {
                let _ = systemctl.run_ok(&["legacy-web-start-enable"]);
                bail!("failed to stop legacy web server");
            }
            legacy_web_stopped = true;
        }

        log("restarting web server");
        if !systemctl.run_ok(&["restart", &config.web_unit]) {
            restore_previous_web(systemctl, legacy_web_stopped);
            bail!("web server restart failed");
        }
        if !systemctl.run_ok(&["enable-web", &config.web_unit]) {
            log("web enable action unavailable; install updated production controls");
        }

        let mut web_ready = false;
        for _ in 0..30 {
            if deps.web.health_ready(&config.web_health_url, release_id) {
                web_ready = true;
                break;
            }
            deps.clock.sleep(Duration::from_secs(1));
        }
        if !web_ready {
            let _ = systemctl.run_ok(&["stop", &config.web_unit]);
            restore_previous_web(systemctl, legacy_web_stopped);
            bail!("web server failed readiness; previous release restored");
        }

        // The restart reset the web server's in-memory registry to the
        // env-file initial (active address only). Republish so the draining
        // list survives deploys that include the web component.
        let draining: Vec<String> = if bot.handoff_pending {
            vec![bot.old_bot_grpc.clone()]
        } else {
            Vec::new()
        };
        if deps
            .web
            .publish_registry(
                &config.web_registry_url,
                &config.registry_secret,
                &grpc_address,
                &draining,
            )
            .is_err()
        {
            log("web server registry unavailable after restart; relying on release env");
        }
    }

    if component_selected(Component::Frontend, components) {
        fsx::ensure_dir_mode(&config.frontend_root, 0o755)?;
        log("publishing frontend assets, HTML, then version metadata");
        let script = worktree_path.join("sakiot_stage/scripts/deploy.sh");
        deps.runner.run(
            &Cmd::new(script.display().to_string())
                .env(
                    "SAKIOT_FRONTEND_ROOT",
                    config.frontend_root.display().to_string(),
                )
                .env(
                    "SAKIOT_FRONTEND_DIST",
                    artifact_dir.join("frontend/dist").display().to_string(),
                ),
        )?;
    }

    // Lines 516-526: finish the old bot's drain once everything is serving.
    if bot.handoff_pending {
        if bot.old_bot_is_legacy {
            systemctl.run(&["legacy-bot-disable", &bot.old_bot_unit])?;
        } else {
            systemctl.run(&["disable", &bot.old_bot_unit])?;
        }
        bot.old_bot_disabled = true;
        deps.admin.shutdown_when_empty(
            &bot.old_bot_grpc,
            &format!("release {release_id} is fully ready"),
        )?;
    }

    // Lines 528-536: only the newest bot unit stays enabled.
    if component_selected(Component::Bot, components) {
        let mut release_dirs: Vec<PathBuf> = std::fs::read_dir(&config.release_root)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.join("fbi-agent").is_dir())
                    .collect()
            })
            .unwrap_or_default();
        release_dirs.sort();
        for dir in release_dirs {
            let Some(stale_release_id) = dir.file_name().map(|n| n.to_string_lossy().into_owned())
            else {
                continue;
            };
            let stale_bot_unit = format!("{}{stale_release_id}.service", config.bot_unit_prefix);
            if stale_bot_unit == bot.new_bot_unit {
                continue;
            }
            let _ = systemctl.run_ok(&["disable", &stale_bot_unit]);
        }
    }

    // Lines 537-539: leave the recovery scope.
    bot.recovery_required = false;
    bot.new_bot_started = false;
    bot.old_bot_disabled = false;
    Ok(())
}

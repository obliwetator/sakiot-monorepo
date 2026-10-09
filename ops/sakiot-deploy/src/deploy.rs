//! The deploy orchestrator. State files and the manifest keep a fixed format
//! so releases already on the server remain valid rollback targets.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::admin_api::AdminApi;
use crate::clock::Clock;
use crate::components::{
    Component, all_components, component_selected, components_for_paths, preview_components,
};
use crate::config::{Config, Mode, Request, Target};
use crate::fsx;
use crate::git;
use crate::lock::DeployLock;
use crate::log;
use crate::promotion::{self, PreparedPromotion};
use crate::release::{
    Manifest, ManifestDatabase, ManifestReused, prune_old_releases, release_id, reusable_artifact,
};
use crate::runner::{Cmd, CommandRunner};
use crate::systemctl::Systemctl;
use crate::validate;
use crate::web_api::WebApi;

mod prepare;
mod services;

use prepare::*;
use services::*;

pub struct Deps<'a> {
    pub runner: &'a dyn CommandRunner,
    pub admin: &'a dyn AdminApi,
    pub web: &'a dyn WebApi,
    pub clock: &'a dyn Clock,
    pub hostname: String,
    /// ops/sakiot-deploy tree OID stamped when the running engine was
    /// installed (ops/update-deploy-engine.sh). None disables the
    /// engine-staleness warning.
    pub engine_src_tree: Option<String>,
    pub free_port: &'a dyn Fn() -> Result<u16>,
    /// Looks a tool up like `command -v`; injectable so tests don't depend on
    /// host PATH.
    pub require_command: &'a dyn Fn(&str) -> Result<()>,
}

/// Looks a tool up on PATH like `command -v`, so the deploy fails up front
/// when a required tool is missing rather than midway through a release.
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

/// The engine binary is installed out-of-band and never built from the
/// release worktree (see ops/README.md), so a release that changes
/// ops/sakiot-deploy still runs on the previously installed engine. Compare
/// the install-time stamp against the release's tree and warn on drift; the
/// deploy itself proceeds, because running on the older engine is the
/// documented behavior.
fn warn_if_engine_stale(deps: &Deps, source_repo: &Path, sha: &str) {
    let Some(installed) = deps.engine_src_tree.as_deref() else {
        return;
    };
    match git::tree_oid(deps.runner, source_repo, sha, "ops/sakiot-deploy") {
        Ok(release_tree) if release_tree == installed => {}
        Ok(release_tree) => {
            log(format!(
                "WARNING: deploy engine is stale: installed from ops/sakiot-deploy tree {installed}, this release carries {release_tree}"
            ));
            log("WARNING: engine changes in this release are NOT running; \
                 refresh with `sudo ops/update-deploy-engine.sh` on the VPS");
        }
        Err(error) => log(format!("engine drift check skipped: {error:#}")),
    }
}

/// `sqlx migrate run` for a deploy without a pre-migrate backup. A preview
/// slot's database starts as a staging snapshot, which can hold migrations
/// the branch does not have yet; sqlx refuses to run past an applied
/// migration it cannot find, so previews ignore those.
fn migrate_run_args(source: &str, target: Target) -> Vec<String> {
    let mut args: Vec<String> = ["migrate", "run", "--source", source]
        .into_iter()
        .map(String::from)
        .collect();
    if target == Target::Preview {
        args.push("--ignore-missing".into());
    }
    args
}

pub fn run(request: &Request, config: &Config, deps: &Deps) -> Result<()> {
    request.validate()?;
    let mode = request.mode;
    let target = request.target;
    let tag = &request.tag;
    let sha = &request.sha;

    // Tool availability. Locking, HTTP, JSON and gRPC happen in-process.
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

    // Directory layout.
    for dir in [
        &config.state_dir,
        &config.state_dir.join("tags"),
        &config.release_root,
        &config.current_root,
        &config.cache_dir,
        &config.promotion_root,
        &config.worktree_root(),
    ] {
        fsx::ensure_dir_mode(dir, 0o750)?;
    }
    // The data directory belongs to the instance's runtime user, which for
    // staging and previews is not the deploy user (install-production.sh,
    // preview-slot.sh), and the services create what they write inside it.
    // Only make sure it exists: changing its mode would fail for another
    // owner and strip the group access the deploy user relies on.
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("failed to create {}", config.data_dir.display()))?;

    // One deploy at a time.
    let _lock = DeployLock::acquire(&config.state_dir.join("deploy.lock"))?;

    // Repository cache, tag verification, tag record.
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
    git::discard_ephemeral_credentials()?;
    git::require_commit(deps.runner, &source_repo, sha)?;
    warn_if_engine_stale(deps, &source_repo, sha);

    let tag_record = config.state_dir.join("tags").join(tag);
    if target == Target::Production {
        validate::validate_tag_record(mode, &tag_record, tag, sha)?;
    }

    let previous_sha = fsx::read_line(&config.state_dir.join("current.sha")).unwrap_or_default();
    let previous_tag = fsx::read_line(&config.state_dir.join("current.tag")).unwrap_or_default();

    // Refuse rollbacks that cross schema changes.
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

    // Release identity.
    let timestamp = crate::clock::compact_timestamp(deps.clock.now_utc())?;
    let release_id = release_id(mode, tag, sha, &timestamp);
    let artifact_dir = config.release_root.join(&release_id);
    let worktree_path = config.worktree_root().join(&release_id);

    // Component selection.
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
    // Preview slots always migrate and deploy the web server and frontend, and
    // never deploy the bot (`preview_components`). Without the bot there is no
    // blue/green bot handoff.
    let components: Vec<Component> = if target == Target::Preview {
        preview_components()
    } else {
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

    // Artifact reuse on rollback or exact staging promotion.
    let mut reuse_bot: Option<PathBuf> = None;
    let mut reuse_web: Option<PathBuf> = None;
    let mut reuse_frontend: Option<PathBuf> = None;
    let mut consumed_promotion: Option<PathBuf> = None;
    if mode == Mode::Release {
        let promoted = promotion::verified(&config.promotion_root, tag, sha)?;
        if request.require_promotion && promoted.is_none() {
            bail!("required staging promotion for {tag} at {sha} was not found");
        }
        if let Some(bundle) = promoted {
            log(format!(
                "verified immutable staging promotion {}",
                bundle.display()
            ));
            consumed_promotion = Some(bundle.clone());
            if component_selected(Component::Bot, &components) {
                reuse_bot = Some(bundle.clone());
            }
            if component_selected(Component::Web, &components) {
                reuse_web = Some(bundle.clone());
            }
            if component_selected(Component::Frontend, &components) {
                reuse_frontend = Some(bundle);
            }
        }
    } else if mode == Mode::Rollback && !config.rollback_force_rebuild {
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

    // Detached worktree at the release SHA, cleaned on exit.
    let worktree = git::Worktree::add(deps.runner, &source_repo, &worktree_path, sha)?;

    // Production and staging intentionally share a Cargo target directory.
    // Their deploy-state locks are separate, so hold an additional common
    // lock across every build and artifact copy. Cargo's own lock ends when
    // `cargo build` exits; without this guard another target could overwrite
    // target/release/web_server before the first deploy copies it.
    let cargo_lock = if build_rust || request.prepare_production_tag.is_some() {
        fsx::ensure_dir_mode(&config.cargo_target_dir, 0o750)?;
        log("acquiring shared Cargo build-and-copy lock");
        Some(DeployLock::acquire(
            &config.cargo_target_dir.join(".sakiot-deploy.lock"),
        )?)
    } else {
        None
    };

    // Test the Rust workspace when building from source.
    let cargo_target = &config.cargo_target_dir;
    if build_rust {
        (deps.require_command)("protoc")?;
        if request.ci_verified {
            log("trusted CI verification received; skipping duplicate Rust tests");
        } else {
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
    }

    // Bot and web binaries (build or reuse).
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
                    .args(["build", "--release", "--locked", "--bin", "fbi_agent"])
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
            let mut args = vec!["build", "--release", "--locked", "--bin", "web_server"];
            if matches!(target, Target::Staging | Target::Preview) {
                args.extend(["--features", "dev-login"]);
            }
            deps.runner.run(
                &Cmd::new("cargo")
                    .args(args)
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

    // Frontend bundle (build or reuse).
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
            let stage_dir = worktree.path().join("sakiot-stage");
            deps.runner.run(
                &Cmd::new("bun")
                    .args(["install", "--frozen-lockfile"])
                    .cwd(&stage_dir),
            )?;
            if !request.ci_verified {
                deps.runner
                    .run(&Cmd::new("bun").args(["run", "test"]).cwd(&stage_dir))?;
            }
            let build_script = if request.ci_verified {
                "build:bundle"
            } else {
                "build"
            };
            deps.runner.run(
                &Cmd::new("bun")
                    .args(["run", build_script])
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

    // A version-bump staging run prepares the production variants on the
    // target Debian host. This avoids both ABI risk from runner-built Linux
    // binaries and the later production compilation cycle.
    let prepared_promotion = if let Some(production_tag) = &request.prepare_production_tag {
        if promotion::verified(&config.promotion_root, production_tag, sha)?.is_some() {
            log(format!(
                "production promotion {production_tag} at {sha} already prepared"
            ));
            None
        } else {
            Some(prepare_production_promotion(
                config,
                deps,
                worktree.path(),
                cargo_target,
                production_tag,
                sha,
            )?)
        }
    } else {
        None
    };
    drop(cargo_lock);

    // ops/tests/run.sh is deliberately not run here: the engine and shims that
    // execute were installed out-of-band, not taken from this release, so
    // testing the release's copies would validate code that is not running.
    // CI runs those suites, and the local `cargo test` above covers the engine
    // when CI did not verify the commit.

    // Migrations, with pre-migrate backup on production.
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
            deps.runner.run(&Cmd::new("sqlx").args(migrate_run_args(
                &migrations_source.display().to_string(),
                target,
            )))?;
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

    // Service handoff under the bot recovery scope.
    let mut bot = BotHandoff::default();
    let release = ServiceRelease {
        target,
        release_id: &release_id,
        components: &components,
        artifact_dir: &artifact_dir,
        worktree_path: worktree.path(),
    };
    let handoff = deploy_services(config, deps, &systemctl, &release, &mut bot);
    if let Err(error) = handoff {
        bot.recover(
            deps,
            &systemctl,
            &config.state_dir,
            &format!("release {release_id} failed: {error}"),
            &config.web_registry_url,
            &config.registry_secret,
        );
        return Err(error);
    }

    // Manifest and state recording.
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
    if let Some(prepared) = prepared_promotion {
        let path = prepared.publish()?;
        log(format!("published production promotion {}", path.display()));
    }
    if let Some(bundle) = consumed_promotion
        && let Err(error) = std::fs::remove_dir_all(&bundle)
    {
        log(format!(
            "consumed promotion cleanup failed for {} ({error})",
            bundle.display()
        ));
    }

    // Garbage collection and the final summary.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_migrate_past_migrations_the_branch_lacks() {
        assert_eq!(
            migrate_run_args("/m", Target::Preview),
            ["migrate", "run", "--source", "/m", "--ignore-missing"]
        );
        assert_eq!(
            migrate_run_args("/m", Target::Staging),
            ["migrate", "run", "--source", "/m"]
        );
    }
}

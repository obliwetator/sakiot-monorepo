//! Putting a built release into service: the bot's blue/green handoff with
//! its recovery path, then the web server's restart and readiness.

use super::*;

/// How long recovery waits for a failed new bot to stop before SIGKILL.
/// A bot without sessions stops in seconds; anything longer is the
/// SIGTERM-hang failure mode, and bot units never time out on their own.
const RECOVERY_STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// Bot blue/green handoff state, shared with the recovery path.
#[derive(Default)]
pub(super) struct BotHandoff {
    recovery_required: bool,
    new_bot_started: bool,
    handoff_pending: bool,
    old_bot_disabled: bool,
    new_bot_unit: String,
    new_bot_grpc: String,
    old_bot_unit: String,
    old_bot_grpc: String,
    previous_bot_unit: String,
    previous_bot_grpc: String,
}

impl BotHandoff {
    /// Tells the old bot to stop draining and keep serving. `reason` names the
    /// actual failure; it is logged and sent as the CancelDrain reason.
    fn cancel_old_drain(&self, deps: &Deps, reason: &str) {
        if !self.recovery_required || self.old_bot_grpc.is_empty() {
            return;
        }
        log(format!("{reason}; cancelling old FBI Agent drain"));
        let _ = deps.admin.cancel_drain(&self.old_bot_grpc, reason);
    }

    /// Undoes a failed handoff: stops and disables the new bot, points the
    /// state files back at the previous bot, re-enables the old unit, cancels
    /// its drain and re-publishes it in the registry. Best-effort: every step
    /// runs even if earlier ones fail. `reason` names the failure that
    /// triggered the unwind.
    pub(super) fn recover(
        &self,
        deps: &Deps,
        systemctl: &Systemctl,
        state_dir: &Path,
        reason: &str,
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
            let _ = systemctl.run_ok(&["enable", &self.old_bot_unit]);
        }
        self.cancel_old_drain(deps, reason);
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

/// Bot blue/green handoff, web swap, frontend publish, and handoff
/// completion. Any error here unwinds through BotHandoff::recover.
/// The built release `deploy_services` puts into service.
pub(super) struct ServiceRelease<'a> {
    pub(super) target: Target,
    pub(super) release_id: &'a str,
    pub(super) components: &'a [Component],
    pub(super) artifact_dir: &'a Path,
    pub(super) worktree_path: &'a Path,
}

pub(super) fn deploy_services(
    config: &Config,
    deps: &Deps,
    systemctl: &Systemctl,
    release: &ServiceRelease,
    bot: &mut BotHandoff,
) -> Result<()> {
    let ServiceRelease {
        target,
        release_id,
        components,
        artifact_dir,
        worktree_path,
    } = *release;
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

        let old_bot_active = !bot.old_bot_unit.is_empty()
            && systemctl.run_ok(&["is-active", "--quiet", &bot.old_bot_unit]);

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
            // No full recovery here: state files and the registry still point
            // at the old bot, so stop the unit, cancel the old drain, and leave
            // them untouched.
            systemctl.stop_bot_bounded(&bot.new_bot_unit, RECOVERY_STOP_TIMEOUT);
            bot.cancel_old_drain(deps, &format!("release {release_id} failed to start"));
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
            // Same shape as the start failure: bypass BotHandoff::recover,
            // since state files and registry are still untouched.
            systemctl.stop_bot_bounded(&bot.new_bot_unit, RECOVERY_STOP_TIMEOUT);
            bot.new_bot_started = false;
            bot.cancel_old_drain(deps, &format!("release {release_id} failed readiness"));
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
        // Preview slots share preview.env, whose PORT is the base value; the
        // per-slot port (derived from the slot name at config load) lands in
        // the release service.env so each slot's web server binds its own.
        let port_override = std::env::var("PORT").ok().filter(|value| !value.is_empty());
        let mut service_env = format!(
            "RELEASE_ID={release_id}\nSAKIOT_DATA_DIR={}\nGRPC_ADDRESS=http://{grpc_address}",
            config.data_dir.display(),
        );
        if let Some(port) = port_override {
            service_env.push_str(&format!("\nPORT={port}"));
        }
        // Slot-correct host variables for preview: the shared preview.env
        // carries the base host, so the web server gets its own subdomain via
        // the release env (later EnvironmentFile wins in the unit). Same for
        // DATABASE_URL: the shared file names the base database, and the
        // per-slot database would otherwise not exist.
        if target == Target::Preview {
            for key in [
                "CORS_ALLOWED_ORIGIN",
                "OAUTH_ALLOWED_OPENER_ORIGINS",
                "DISCORD_REDIRECT_URI",
                "DATABASE_URL",
            ] {
                if let Ok(value) = std::env::var(key) {
                    service_env.push_str(&format!("\n{key}={value}"));
                }
            }
        }
        fsx::write_line(&artifact_dir.join("web/service.env"), &service_env)?;
        std::fs::set_permissions(
            artifact_dir.join("web/service.env"),
            std::fs::Permissions::from_mode(0o640),
        )?;

        let web_link = config.current_root.join("web");
        let previous_web_target = std::fs::read_link(&web_link).ok();
        let restore_previous_web = |systemctl: &Systemctl| {
            if let Some(previous) = &previous_web_target {
                let _ = fsx::atomic_symlink(previous, &web_link);
                let _ = systemctl.run_ok(&["restart", &config.web_unit]);
            }
        };

        fsx::atomic_symlink(&artifact_dir.join("web"), &web_link)?;

        log("restarting web server");
        if !systemctl.run_ok(&["restart", &config.web_unit]) {
            restore_previous_web(systemctl);
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
            restore_previous_web(systemctl);
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
        let script = worktree_path.join("sakiot-stage/scripts/deploy.sh");
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

    // Finish the old bot's drain once everything is serving.
    if bot.handoff_pending {
        systemctl.run(&["disable", &bot.old_bot_unit])?;
        bot.old_bot_disabled = true;
        deps.admin.shutdown_when_empty(
            &bot.old_bot_grpc,
            &format!("release {release_id} is fully ready"),
        )?;
    }

    // Only the newest bot unit stays enabled.
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

    // Leave the recovery scope.
    bot.recovery_required = false;
    bot.new_bot_started = false;
    bot.old_bot_disabled = false;
    Ok(())
}

//! Preparing, during a version-bump staging deploy, the production bundle
//! that the tagged release later promotes.

use super::*;

pub(super) fn prepare_production_promotion(
    config: &Config,
    deps: &Deps,
    worktree: &Path,
    cargo_target: &Path,
    tag: &str,
    sha: &str,
) -> Result<PreparedPromotion> {
    let version = workspace_version(&worktree.join("Cargo.toml"))?;
    if tag.strip_prefix('v') != Some(version.as_str()) {
        bail!("production promotion tag {tag} does not match workspace version {version}");
    }

    (deps.require_command)("protoc")?;
    (deps.require_command)("bun")?;
    log(format!("preparing immutable production bundle for {tag}"));
    let prepared = PreparedPromotion::new(&config.promotion_root, tag, sha)?;
    fsx::ensure_dir_mode(&prepared.bot_dir(), 0o755)?;
    fsx::ensure_dir_mode(&prepared.web_dir(), 0o755)?;
    fsx::ensure_dir_mode(&prepared.frontend_dir(), 0o755)?;

    deps.runner.run(
        &Cmd::new("cargo")
            .args([
                "build",
                "--release",
                "--locked",
                "--bin",
                "fbi_agent",
                "--bin",
                "web_server",
            ])
            .cwd(worktree)
            .env("SQLX_OFFLINE", "true")
            .env("CARGO_TARGET_DIR", cargo_target.display().to_string()),
    )?;
    fsx::install_file(
        &cargo_target.join("release/fbi_agent"),
        &prepared.bot_dir().join("fbi_agent"),
        0o755,
    )?;
    fsx::install_file(
        &cargo_target.join("release/web_server"),
        &prepared.web_dir().join("web_server"),
        0o755,
    )?;

    let production_env = crate::config::plain_env_vars(&config.production_env_file)?;
    if !production_env.contains_key("VITE_API_URL") {
        bail!(
            "set VITE_API_URL in {} before preparing production",
            config.production_env_file.display()
        );
    }
    let stage_dir = worktree.join("sakiot-stage");
    deps.runner.run(
        &Cmd::new("bun")
            .args(["install", "--frozen-lockfile"])
            .cwd(&stage_dir),
    )?;
    let mut build = Cmd::new("bun")
        .args(["run", "build:bundle"])
        .cwd(&stage_dir)
        .env("SAKIOT_RELEASE_TAG", tag)
        .env("SAKIOT_COMMIT_SHA", sha)
        .env(
            "SAKIOT_BUNDLE_VERSION",
            format!("{tag}-{}", &sha[..12.min(sha.len())]),
        );
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("VITE_")) {
        build = build.env_remove(key);
    }
    for (key, value) in production_env
        .iter()
        .filter(|(key, _)| key.starts_with("VITE_"))
    {
        build = build.env(key, value);
    }
    deps.runner.run(&build)?;
    deps.runner.run(&Cmd::new("cp").arg("-a").args([
        stage_dir.join("dist").display().to_string(),
        prepared.frontend_dir().join("dist").display().to_string(),
    ]))?;
    Ok(prepared)
}

fn workspace_version(manifest: &Path) -> Result<String> {
    let content = std::fs::read_to_string(manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let mut in_workspace_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace_package = trimmed == "[workspace.package]";
            continue;
        }
        if in_workspace_package
            && let Some(value) = trimmed.strip_prefix("version")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            let version = value.trim().trim_matches('"');
            if !version.is_empty() {
                return Ok(version.to_string());
            }
        }
    }
    bail!(
        "workspace package version missing from {}",
        manifest.display()
    )
}

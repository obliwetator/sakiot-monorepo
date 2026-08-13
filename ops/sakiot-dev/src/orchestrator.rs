//! Top-level command orchestration.

use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::{
    Command, DbCommand, Destination, FixtureFetchArgs, FixtureStartup, FixtureSyncArgs, Source,
};
use crate::config::{Config, Overrides};
use crate::db::DatabaseFactory;
use crate::dependencies::{self, DependencySet};
use crate::environment;
use crate::fixtures::destination::{import_local, import_staging};
use crate::fixtures::export::{export_bundle, fixture_server_counts};
use crate::fixtures::selection::{BulkSelection, CountSelection, parse_selector};
use crate::prompt::{self, PromptIo};
use crate::runner::{Cmd, CommandRunner};
use crate::supervisor::{self, ServiceSupervisor};

pub struct Deps<'a> {
    pub runner: &'a dyn CommandRunner,
    pub databases: &'a dyn DatabaseFactory,
    pub prompt: &'a mut dyn PromptIo,
}

pub async fn run(cli: crate::cli::Cli, root: &Path, deps: Deps<'_>) -> Result<()> {
    let services = supervisor::RealServiceSupervisor;
    run_with_services(cli, root, deps, &services).await
}

pub async fn run_with_services(
    cli: crate::cli::Cli,
    root: &Path,
    deps: Deps<'_>,
    services: &dyn ServiceSupervisor,
) -> Result<()> {
    match cli.command {
        Command::Up(args) => run_up(args.fixtures, root, deps, services).await,
        Command::Db { command } => match command {
            DbCommand::Up => run_db_up(root, deps, true).await.map(|_| ()),
            DbCommand::Down => run_db_down(root, deps.runner),
            DbCommand::Reset(args) => run_reset(root, deps, args.yes).await,
        },
        Command::Fixtures { command } => match command {
            crate::cli::FixturesCommand::Sync(args) => run_sync(root, deps, args).await,
            crate::cli::FixturesCommand::Fetch(args) => run_fetch(root, deps, args).await,
        },
        Command::Clean(args) => run_clean(root, deps, args.yes).await,
        Command::Completions(_) => bail!("completion generation must be handled by the CLI binary"),
    }
}

async fn run_up(
    requested: Option<FixtureStartup>,
    root: &Path,
    mut deps: Deps<'_>,
    services: &dyn ServiceSupervisor,
) -> Result<()> {
    let initial = Config::load(root.to_path_buf(), Overrides::default())?;
    if requested.is_none() && !deps.prompt.is_terminal() {
        bail!("non-interactive cargo dev up must specify --fixtures skip or --fixtures full")
    }
    let policy = prompt::startup_policy(deps.prompt, requested.or(Some(initial.startup_fixtures)))?;
    let config = ensure_environment(root, Overrides::default())?;
    dependencies::check(deps.runner, DependencySet::Up, false)?;
    let started = run_db_up_with_config(&config, &mut deps, true).await?;

    let fixture_result = match policy {
        FixtureStartup::Skip => Ok(()),
        FixtureStartup::Full => {
            let bulk = BulkSelection {
                source: config.source,
                guild: Some(config.fixture_guild),
                ..BulkSelection::default()
            };
            run_fixture_sync_with_config(&config, &mut deps, bulk).await
        }
        FixtureStartup::Custom => {
            let (recordings, clips, stamps) = prompt::custom_counts(deps.prompt)?;
            if recordings.is_none() && clips.is_none() && stamps.is_none() {
                Ok(())
            } else {
                run_fixture_sync_with_config(
                    &config,
                    &mut deps,
                    BulkSelection {
                        recordings,
                        clips,
                        days: None,
                        stamps,
                        source: config.source,
                        guild: Some(config.fixture_guild),
                    },
                )
                .await
            }
        }
        FixtureStartup::Prompt => unreachable!("startup prompt is resolved before orchestration"),
    };
    if let Err(error) = fixture_result {
        warn(format!(
            "fixture startup failed; continuing without fixtures: {error:#}"
        ));
    }

    let services = services.supervise(&config).await;
    if started {
        let _ = compose_command(&config, ["stop"]).map(|command| deps.runner.run(&command));
    }
    services
}

async fn run_db_up(root: &Path, mut deps: Deps<'_>, report: bool) -> Result<bool> {
    let config = ensure_environment(root, Overrides::default())?;
    dependencies::check(deps.runner, DependencySet::Database, false)?;
    run_db_up_with_config(&config, &mut deps, report).await
}

async fn run_db_up_with_config(config: &Config, deps: &mut Deps<'_>, report: bool) -> Result<bool> {
    if !config.local_database_is_managed() {
        bail!(
            "DATABASE_URL points at {}; cargo dev manages {} through compose.dev.yml",
            config.database_url,
            crate::config::DEFAULT_LOCAL_DATABASE_URL
        )
    }
    let was_running = compose_running(config, deps.runner);
    deps.runner
        .run(&compose_command(config, ["up", "-d", "--wait"])?)?;
    let database = deps.databases.connect(&config.database_url).await?;
    database.migrate().await?;
    database.seed(config.dev_account_id).await?;
    environment::prepare_media_dirs(&config.data_dir)?;
    if report {
        log("local PostgreSQL is migrated and seeded");
    }
    Ok(!was_running)
}

fn run_db_down(root: &Path, runner: &dyn CommandRunner) -> Result<()> {
    let config = Config::load(root.to_path_buf(), Overrides::default())?;
    dependencies::check(runner, DependencySet::Database, false)?;
    runner.run(&compose_command(&config, ["down"])?)?;
    Ok(())
}

async fn run_reset(root: &Path, mut deps: Deps<'_>, yes: bool) -> Result<()> {
    let config = ensure_environment(root, Overrides::default())?;
    prompt::confirm(
        deps.prompt,
        "This drops the local PostgreSQL volume and managed fixture media",
        yes,
    )?;
    dependencies::check(deps.runner, DependencySet::Database, false)?;
    deps.runner
        .run(&compose_command(&config, ["down", "-v"])?)?;
    clear_manifest(&config)?;
    let _ = run_db_up_with_config(&config, &mut deps, true).await?;
    Ok(())
}

async fn run_clean(root: &Path, deps: Deps<'_>, yes: bool) -> Result<()> {
    let config = Config::load(root.to_path_buf(), Overrides::default())?;
    prompt::confirm(
        deps.prompt,
        "This drops the local PostgreSQL volume and removes only manifest-tracked fixture files",
        yes,
    )?;
    dependencies::check(deps.runner, DependencySet::Database, false)?;
    deps.runner
        .run(&compose_command(&config, ["down", "-v"])?)?;
    clear_manifest(&config)?;
    log("clean done; .env and unrelated media were retained");
    Ok(())
}

async fn run_sync(root: &Path, mut deps: Deps<'_>, args: FixtureSyncArgs) -> Result<()> {
    let config = ensure_environment(root, Overrides::default())?;
    let source = args.source.unwrap_or(config.source);
    let guild = args.guild.unwrap_or(config.fixture_guild);
    let has_explicit_selection = args.recordings.is_some()
        || args.clips.is_some()
        || args.days.is_some()
        || args.stamps.is_some();
    if !has_explicit_selection && !deps.prompt.is_terminal() {
        bail!(
            "non-interactive fixture sync must specify --recordings, --clips, --stamps, or --days"
        )
    }
    let ssh = resolve_ssh(&config, source, deps.prompt)?;
    dependencies::check(deps.runner, DependencySet::Fixtures, ssh != "local")?;
    let counts = fixture_server_counts(deps.runner, &config, source, &ssh, guild, args.days)?;
    if let Some(days) = args.days {
        log(format!(
            "{} guild {} has {} recording(s), {} clip(s), and {} stamp(s) from the last {days} day(s)",
            source.label(),
            guild,
            counts.recordings,
            counts.clips,
            counts.stamps
        ));
    } else {
        log(format!(
            "{} guild {} has {} finalized recording(s), {} clip(s), and {} stamp(s) on the server",
            source.label(),
            guild,
            counts.recordings,
            counts.clips,
            counts.stamps
        ));
    }
    let bulk = if args.days.is_some() {
        BulkSelection {
            recordings: CountSelection::All,
            clips: CountSelection::All,
            days: args.days,
            stamps: CountSelection::All,
            guild: Some(guild),
            source,
        }
    } else {
        let recordings = match args.recordings {
            Some(selection) => selection,
            None if has_explicit_selection => CountSelection::None,
            None => prompt::latest_recording_count(deps.prompt, counts.recordings)?,
        };
        BulkSelection {
            recordings,
            clips: args.clips.unwrap_or(CountSelection::None),
            days: None,
            stamps: args.stamps.unwrap_or(CountSelection::None),
            guild: Some(guild),
            source,
        }
    };
    if bulk.recordings.is_none() && bulk.clips.is_none() && bulk.stamps.is_none() {
        log("nothing selected; leaving local fixtures unchanged");
        return Ok(());
    }
    run_fixture_sync_with_ssh(&config, &mut deps, bulk, &ssh).await
}

async fn run_fetch(root: &Path, mut deps: Deps<'_>, args: FixtureFetchArgs) -> Result<()> {
    let config = ensure_environment(root, Overrides::default())?;
    let (selector_input, kind) = args.normalized_selector();
    if selector_input.is_empty() {
        bail!("fixtures fetch requires a selector")
    }
    let selector = parse_selector(selector_input, kind)?;
    let source = args
        .source
        .or_else(|| selector.source_hint())
        .unwrap_or(config.source);
    if args.destination == Destination::Staging && source != Source::Production {
        bail!(
            "--destination staging copies production data into staging, but the source is {}",
            source.label()
        )
    }
    let ssh = resolve_ssh(&config, source, deps.prompt)?;
    dependencies::check(deps.runner, DependencySet::Fixtures, ssh != "local")?;
    if args.destination == Destination::Local {
        ensure_database_running(&config, &mut deps).await?;
    }
    let bundle = export_bundle(deps.runner, &config, source, &ssh, None, Some(&selector))?;
    match args.destination {
        Destination::Local => import_local(deps.runner, deps.databases, &config, bundle).await,
        Destination::Staging => {
            import_staging(
                deps.runner,
                &config,
                bundle,
                selector_input,
                args.yes,
                deps.prompt,
                &ssh,
            )
            .await
        }
    }
}

async fn run_fixture_sync_with_config(
    config: &Config,
    deps: &mut Deps<'_>,
    bulk: BulkSelection,
) -> Result<()> {
    let ssh = resolve_ssh(config, bulk.source, deps.prompt)?;
    run_fixture_sync_with_ssh(config, deps, bulk, &ssh).await
}

async fn run_fixture_sync_with_ssh(
    config: &Config,
    deps: &mut Deps<'_>,
    bulk: BulkSelection,
    ssh: &str,
) -> Result<()> {
    dependencies::check(deps.runner, DependencySet::Fixtures, ssh != "local")?;
    ensure_database_running(config, deps).await?;
    let bundle = export_bundle(deps.runner, config, bulk.source, ssh, Some(&bulk), None)?;
    import_local(deps.runner, deps.databases, config, bundle).await
}

async fn ensure_database_running(config: &Config, deps: &mut Deps<'_>) -> Result<()> {
    if !compose_running(config, deps.runner) {
        let _ = run_db_up_with_config(config, deps, false).await?;
    }
    Ok(())
}

fn ensure_environment(root: &Path, overrides: Overrides) -> Result<Config> {
    let initial = Config::load(root.to_path_buf(), overrides.clone())?;
    environment::ensure_local_environment(root, &initial)?;
    Config::load(root.to_path_buf(), overrides)
}

fn resolve_ssh<P: PromptIo + ?Sized>(
    config: &Config,
    source: Source,
    prompt: &mut P,
) -> Result<String> {
    if let Some(ssh) = &config.ssh {
        return Ok(if ssh.is_empty() {
            "local".into()
        } else {
            ssh.clone()
        });
    }
    let source_data = config.remote(source).data_dir;
    if source_data.is_dir() {
        return Ok("local".into());
    }
    if !prompt.is_terminal() {
        bail!("set SAKIOT_DEV_SSH=user@host or use local source data in non-interactive use")
    }
    let ssh = prompt.ask("VPS SSH target (user@host): ")?;
    if ssh.trim().is_empty() {
        bail!("an SSH target is required")
    }
    Ok(ssh)
}

fn compose_running(config: &Config, runner: &dyn CommandRunner) -> bool {
    compose_command(config, ["ps", "--status", "running", "--services"])
        .ok()
        .and_then(|command| runner.run_capture(&command).ok())
        .is_some_and(|output| output.lines().any(|line| line.trim() == "postgres"))
}

fn compose_command<const N: usize>(config: &Config, args: [&str; N]) -> Result<Cmd> {
    Ok(Cmd::new("docker").args(
        std::iter::once("compose".to_string())
            .chain(std::iter::once("-f".to_string()))
            .chain(std::iter::once(config.compose_file.display().to_string()))
            .chain(args.into_iter().map(str::to_string)),
    ))
}

fn clear_manifest(config: &Config) -> Result<()> {
    let manifest = crate::fixtures::manifest::ManagedManifest::load(&config.data_dir)?;
    let removed = manifest.clear(&config.data_dir)?;
    log(format!("removed {removed} managed fixture file(s)"));
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "local development progress is a user-facing CLI boundary"
)]
fn log(message: impl AsRef<str>) {
    println!("[dev] {}", message.as_ref());
}

#[expect(
    clippy::print_stderr,
    reason = "warnings must remain visible alongside command errors"
)]
fn warn(message: impl AsRef<str>) {
    eprintln!("[dev] warning: {}", message.as_ref());
}

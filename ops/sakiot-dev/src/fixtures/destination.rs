//! Destination import, managed-media updates, staging publishing, and checks.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

use crate::cli::Source;
use crate::config::Config;
use crate::db::{DatabaseFactory, LocalDatabase};
use crate::environment::prepare_media_dirs;
use crate::fixtures::export::FixtureBundle;
use crate::fixtures::import::{build_import_batch, build_prune_batch};
use crate::fixtures::manifest::ManagedManifest;
use crate::fixtures::remote::{RemoteSql, quote_shell};
use crate::runner::{Cmd, CommandRunner};

pub async fn import_local<R: CommandRunner + ?Sized, F: DatabaseFactory + ?Sized>(
    runner: &R,
    factory: &F,
    config: &Config,
    bundle: FixtureBundle,
) -> Result<()> {
    prepare_media_dirs(&config.data_dir)?;
    let database = factory.connect(&config.database_url).await?;
    let old_manifest = ManagedManifest::load(&config.data_dir)?;
    let batch = build_import_batch(database.as_ref(), &bundle.workspace, None).await?;
    database.run_batch(&batch).await?;
    grant_local_access(database.as_ref(), &bundle.guild_ids, config.dev_account_id).await?;

    if bundle.replace_recordings && !old_manifest.recordings.is_empty() {
        let keep = read_lines(&bundle.workspace, "new-recordings.list")?;
        let managed = old_manifest.recordings.iter().cloned().collect::<Vec<_>>();
        let prune = build_prune_batch(database.as_ref(), &keep, &managed).await?;
        database.run_batch(&prune).await?;
    }

    install_media(runner, &bundle.workspace.media, &config.data_dir)?;
    let new_files = read_lines(&bundle.workspace, "new-files.list")?;
    let new_recordings = read_lines(&bundle.workspace, "new-recordings.list")?;
    let old_files = old_manifest.files.clone();
    let mut manifest = if bundle.replace_recordings {
        ManagedManifest::default()
    } else {
        old_manifest
    };
    if bundle.replace_recordings {
        for old in old_files {
            if !new_files.iter().any(|new| Path::new(new) == old) {
                let path = crate::fixtures::manifest::safe_join(&config.data_dir, &old)?;
                if path.is_file() {
                    fs::remove_file(path)?;
                }
            }
        }
        manifest.files.clear();
    }
    manifest
        .files
        .extend(new_files.into_iter().map(PathBuf::from));
    if bundle.replace_recordings {
        manifest.recordings = new_recordings.into_iter().collect();
    } else {
        manifest.recordings.extend(new_recordings);
    }
    manifest.store(&config.data_dir)?;
    log(format!(
        "done: {} recording(s), {} clip(s), {} stamp(s) imported from {}",
        bundle.summary.recordings,
        bundle.summary.clips,
        bundle.summary.stamps,
        bundle.source.label()
    ));
    Ok(())
}

pub async fn import_staging<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    bundle: FixtureBundle,
    selector_input: &str,
    assume_yes: bool,
    prompt: &mut dyn crate::prompt::PromptIo,
    ssh: &str,
) -> Result<()> {
    if bundle.source != Source::Production {
        bail!("staging imports must read from production; pass --source production")
    }
    confirm_staging(prompt, &bundle, config, ssh, assume_yes)?;
    let destination =
        RemoteSql::configure_write(runner, config, &config.remote(Source::Staging), ssh)?;
    let marker = import_marker(selector_input)?;
    let batch = build_import_batch(&destination, &bundle.workspace, Some(&marker)).await?;
    destination.execute(&batch.render_psql())?;

    let account = staging_account_id(config, &destination)?;
    let guilds = numeric_list(&bundle.guild_ids);
    let access_sql = if let Some(account) = account {
        format!(
            "INSERT INTO guilds_present (guild_id)
                 SELECT id FROM guilds WHERE id IN ({guilds}) ON CONFLICT DO NOTHING;
             INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
                 SELECT id, {account}, 'Imported from production ' || id, NULL, true, 8, '{{}}'
                   FROM guilds WHERE id IN ({guilds}) ON CONFLICT DO NOTHING;"
        )
    } else {
        format!(
            "INSERT INTO guilds_present (guild_id)
                 SELECT id FROM guilds WHERE id IN ({guilds}) ON CONFLICT DO NOTHING;"
        )
    };
    destination.execute(access_sql.as_bytes())?;
    if account.is_none() {
        log(
            "warning: could not read staging DEV_ACCOUNT_ID; imported guilds are not granted to dev_login",
        );
    }

    let fragment = import_manifest_fragment(&bundle, selector_input)?;
    publish_to_staging(runner, config, ssh, &bundle.workspace.media, &fragment)?;
    if let Err(error) = verify_staging(&destination, config, &bundle, account).await {
        log(format!(
            "WARNING: staging verification failed: {error}; already-imported data was not rolled back"
        ));
        bail!("staging verification failed; already-imported data was not rolled back")
    }
    log(format!(
        "done: {} recording(s), {} clip(s), {} stamp(s) copied into staging",
        bundle.summary.recordings, bundle.summary.clips, bundle.summary.stamps
    ));
    Ok(())
}

async fn grant_local_access(
    database: &dyn LocalDatabase,
    guilds: &[i64],
    account: i64,
) -> Result<()> {
    if guilds.is_empty() {
        return Ok(());
    }
    let guilds = numeric_list(guilds);
    database
        .execute(&format!(
            "INSERT INTO guilds_present (guild_id)
                 SELECT id FROM guilds WHERE id IN ({guilds}) ON CONFLICT DO NOTHING;
             INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
                 SELECT id, {account}, 'Fixture guild ' || id, NULL, true, 8, '{{}}'
                   FROM guilds WHERE id IN ({guilds}) ON CONFLICT DO NOTHING;"
        ))
        .await
}

fn install_media<R: CommandRunner + ?Sized>(
    runner: &R,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    fs::create_dir_all(destination)?;
    runner.run(&Cmd::new("rsync").args([
        "-a",
        &format!("{}/", source.display()),
        &format!("{}/", destination.display()),
    ]))
}

fn confirm_staging<P: crate::prompt::PromptIo + ?Sized>(
    prompt: &mut P,
    bundle: &FixtureBundle,
    config: &Config,
    ssh: &str,
    assume_yes: bool,
) -> Result<()> {
    let message = format!(
        "COPYING PRODUCTION DATA INTO STAGING ({} recording(s), {} clip(s), {} stamp(s); database {} on {ssh}, media {})",
        bundle.summary.recordings,
        bundle.summary.clips,
        bundle.summary.stamps,
        config.staging_db,
        config.staging_data.display()
    );
    crate::prompt::confirm(prompt, &message, assume_yes)
}

fn staging_account_id<R: CommandRunner + ?Sized>(
    config: &Config,
    destination: &RemoteSql<'_, R>,
) -> Result<Option<i64>> {
    if let Some(account) = config.staging_account_id {
        return Ok(Some(account));
    }
    let read_env = format!(
        "set -a; . {}; set +a; printf '%s' \"$DEV_ACCOUNT_ID\"",
        quote_shell(&config.staging_env_file.display().to_string())
    );
    let account = destination
        .remote_capture(&read_env)
        .or_else(|_| {
            destination.remote_capture(&format!(
                "sudo -n -u sakiot /bin/bash -lc {}",
                quote_shell(&read_env)
            ))
        })
        .ok();
    let Some(account) = account else {
        return Ok(None);
    };
    let account = account.trim();
    if account.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        account
            .parse::<i64>()
            .context("staging DEV_ACCOUNT_ID is not an integer")?,
    ))
}

fn import_marker(selector: &str) -> Result<String> {
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .context("could not format import timestamp")?;
    let who = format!(
        "{}@{}",
        std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
        std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
    );
    Ok(format!(
        "{{\"source\":\"production\",\"origin_url\":{},\"imported_at\":{},\"imported_by\":{},\"tool\":\"cargo dev\"}}",
        json_string(selector),
        json_string(&now),
        json_string(&who)
    ))
}

fn import_manifest_fragment(bundle: &FixtureBundle, selector: &str) -> Result<Vec<u8>> {
    let stamp = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .context("could not format import timestamp")?;
    let who = format!(
        "{}@{}",
        std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
        std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
    );
    let mut output = String::new();
    for line in read_lines(&bundle.workspace, "audio_files.tsv")? {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 2 {
            output.push_str(&format!(
                "{stamp}\tfile_name={}\tguild={}\tby={who}\tfrom={selector}\n",
                fields[0], fields[1]
            ));
        }
    }
    for line in read_lines(&bundle.workspace, "clip_meta.tsv")? {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 2 {
            output.push_str(&format!(
                "{stamp}\tclip_id={}\tguild={}\tby={who}\tfrom={selector}\n",
                fields[0], fields[1]
            ));
        }
    }
    for line in read_lines(&bundle.workspace, "stamp_meta.tsv")? {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 6 {
            output.push_str(&format!(
                "{stamp}\tstamp=origin:{}/ts:{}\tguild={}\tby={who}\tfrom={selector}\n",
                fields[0], fields[5], fields[1]
            ));
        }
    }
    Ok(output.into_bytes())
}

fn publish_to_staging<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    ssh: &str,
    media: &Path,
    fragment: &[u8],
) -> Result<()> {
    let destination = &config.staging_data;
    let manifest = destination.join(&config.import_manifest);
    let writable = remote_ok(
        runner,
        ssh,
        &format!(
            "test -w {}",
            quote_shell(&destination.display().to_string())
        ),
    );
    if writable && publish_direct(runner, ssh, media, destination, &manifest, fragment).is_ok() {
        return Ok(());
    }
    if remote_ok(runner, ssh, "sudo -n -u sakiot true") {
        publish_with_rsync_path(
            runner,
            ssh,
            media,
            destination,
            &manifest,
            fragment,
            &config.staging_rsync_path,
        )?;
        return Ok(());
    }
    publish_via_stage(runner, ssh, media, destination, &manifest, fragment)
}

fn publish_direct<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    media: &Path,
    destination: &Path,
    manifest: &Path,
    fragment: &[u8],
) -> Result<()> {
    let source = format!("{}/", media.display());
    let target = if ssh == "local" {
        format!("{}/", destination.display())
    } else {
        format!("{}:{}/", ssh, destination.display())
    };
    runner.run(&Cmd::new("rsync").args(["-a", &source, &target]))?;
    append_remote_file(runner, ssh, manifest, fragment)
}

fn publish_with_rsync_path<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    media: &Path,
    destination: &Path,
    manifest: &Path,
    fragment: &[u8],
    rsync_path: &str,
) -> Result<()> {
    let source = format!("{}/", media.display());
    let target = if ssh == "local" {
        format!("{}/", destination.display())
    } else {
        format!("{}:{}/", ssh, destination.display())
    };
    runner.run(&Cmd::new("rsync").args([
        "-a",
        &format!("--rsync-path={rsync_path}"),
        &source,
        &target,
    ]))?;
    append_remote_file(runner, ssh, manifest, fragment)
}

fn publish_via_stage<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    media: &Path,
    destination: &Path,
    manifest: &Path,
    fragment: &[u8],
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "staging media needs sudo, but no terminal is available; grant group write access or run the staged sudo command interactively"
        )
    }
    let stage = tempfile::tempdir().context("could not create staging payload")?;
    let stage_path = stage.path();
    let stage_media = stage_path.join("media");
    fs::create_dir_all(&stage_media)?;
    runner.run(&Cmd::new("rsync").args([
        "-a",
        &format!("{}/", media.display()),
        &format!("{}/", stage_media.display()),
    ]))?;
    let fragment_path = stage_path.join(".import-manifest");
    fs::write(&fragment_path, fragment)?;
    if ssh == "local" {
        let install = format!(
            "rsync -a {} {} && cat {} >> {}",
            quote_shell(&format!("{}/", stage_media.display())),
            quote_shell(&format!("{}/", destination.display())),
            quote_shell(&fragment_path.display().to_string()),
            quote_shell(&manifest.display().to_string())
        );
        runner.run(&Cmd::new("sudo").args(["-u", "sakiot", "/bin/sh", "-lc", &install]))?;
    } else {
        let remote_stage = runner
            .run_capture(&Cmd::new("ssh").args([
                ssh,
                "d=$(mktemp -d /tmp/sakiot-dev-import.XXXXXX) && chmod 0755 \"$d\" && printf %s \"$d\"",
            ]))?
            .trim()
            .to_string();
        runner.run(&Cmd::new("rsync").args([
            "-a",
            &format!("{}/", stage_path.display()),
            &format!("{ssh}:{remote_stage}/"),
        ]))?;
        let install = format!(
            "rsync -a {} {} && cat {} >> {}",
            quote_shell(&format!("{remote_stage}/media/")),
            quote_shell(&format!("{}/", destination.display())),
            quote_shell(&format!("{remote_stage}/.import-manifest")),
            quote_shell(&manifest.display().to_string())
        );
        let install_result = runner.run(&Cmd::new("ssh").args([
            "-t",
            ssh,
            &format!("sudo -u sakiot /bin/sh -lc {}", quote_shell(&install)),
        ]));
        let cleanup_result = runner.run_ok(
            &Cmd::new("ssh").args([ssh, &format!("rm -rf {}", quote_shell(&remote_stage))]),
        );
        install_result?;
        if !cleanup_result {
            log("warning: could not remove the temporary remote fixture stage");
        }
    }
    Ok(())
}

fn append_remote_file<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    path: &Path,
    fragment: &[u8],
) -> Result<()> {
    let command = format!("cat >> {}", quote_shell(&path.display().to_string()));
    if ssh == "local" {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("could not open staging manifest {}", path.display()))?;
        file.write_all(fragment)
            .with_context(|| format!("could not append staging manifest {}", path.display()))?;
    } else {
        runner.run(&Cmd::new("ssh").args([ssh, &command]).input(fragment))?;
    }
    Ok(())
}

fn remote_ok<R: CommandRunner + ?Sized>(runner: &R, ssh: &str, command: &str) -> bool {
    if ssh == "local" {
        runner.run_ok(&Cmd::new("sh").args(["-lc", command]))
    } else {
        runner.run_ok(&Cmd::new("ssh").args([ssh, command]))
    }
}

async fn verify_staging<R: CommandRunner + ?Sized>(
    destination: &RemoteSql<'_, R>,
    config: &Config,
    bundle: &FixtureBundle,
    account: Option<i64>,
) -> Result<()> {
    let files = read_lines(&bundle.workspace, "new-files.list")?;
    let missing =
        crate::fixtures::remote::remote_missing_files(destination, &config.staging_data, &files)?;
    let mut failures = Vec::new();
    if !missing.is_empty() {
        failures.push(format!("{} media file(s) are missing", missing.len()));
    }
    let clip_ids = read_lines(&bundle.workspace, "clip-ids.list")?;
    if !clip_ids.is_empty() {
        let count = destination.scalar(&format!(
            "SELECT count(*) FROM clips WHERE clip_id IN ({}) AND deleted_at IS NULL",
            clip_ids
                .iter()
                .map(|id| format!("'{}'", id.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",")
        ))?;
        if count.parse::<usize>().unwrap_or(0) < clip_ids.len() {
            failures.push(format!(
                "only {count} of {} clip rows are present",
                clip_ids.len()
            ));
        }
    }
    let stamp_pairs = read_lines(&bundle.workspace, "stamp_meta.tsv")?
        .iter()
        .filter_map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            let guild = fields.get(1)?.parse::<i64>().ok()?;
            let stamp_ts = fields.get(5)?.parse::<i64>().ok()?;
            Some(format!("({guild}, {stamp_ts})"))
        })
        .collect::<Vec<_>>();
    if !stamp_pairs.is_empty() {
        let count = destination.scalar(&format!(
            "SELECT count(*) FROM stamps WHERE (guild_id, stamp_ts) IN ({})",
            stamp_pairs.join(",")
        ))?;
        if count.parse::<usize>().unwrap_or(0) < stamp_pairs.len() {
            failures.push(format!(
                "only {count} of {} stamp rows are present",
                stamp_pairs.len()
            ));
        }
    }
    for (label, query) in [
        (
            "guilds_present",
            format!(
                "SELECT count(*) FROM guilds_present WHERE guild_id IN ({})",
                numeric_list(&bundle.guild_ids)
            ),
        ),
        (
            "everyone roles",
            format!(
                "SELECT count(*) FROM roles WHERE guild_id IN ({}) AND role_id = guild_id",
                numeric_list(&bundle.guild_ids)
            ),
        ),
        (
            "voice channels",
            format!(
                "SELECT count(*) FROM channels WHERE guild_id IN ({}) AND type = 2",
                numeric_list(&bundle.guild_ids)
            ),
        ),
        (
            "channel permissions",
            format!(
                "SELECT count(*) FROM channel_permissions cp JOIN channels c ON c.channel_id = cp.channel_id WHERE c.guild_id IN ({})",
                numeric_list(&bundle.guild_ids)
            ),
        ),
        (
            "member roles",
            format!(
                "SELECT count(*) FROM user_roles ur JOIN roles r ON r.role_id = ur.role_id WHERE r.guild_id IN ({})",
                numeric_list(&bundle.guild_ids)
            ),
        ),
    ] {
        if destination.scalar(&query)?.parse::<usize>().unwrap_or(0) == 0 {
            failures.push(format!("{label} are absent"));
        }
    }
    if let Some(account) = account {
        let count = destination.scalar(&format!(
            "SELECT count(*) FROM user_guilds WHERE user_id = {account} AND id IN ({})",
            numeric_list(&bundle.guild_ids)
        ))?;
        if count.parse::<usize>().unwrap_or(0) < bundle.guild_ids.len() {
            failures.push(format!(
                "dev_login account {account} cannot see all imported guilds"
            ));
        }
    } else {
        failures.push("staging DEV_ACCOUNT_ID could not be read for dev_login reachability".into());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

fn numeric_list(values: &[i64]) -> String {
    if values.is_empty() {
        "-1".into()
    } else {
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn json_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn read_lines(
    workspace: &crate::fixtures::workspace::FixtureWorkspace,
    name: &str,
) -> Result<Vec<String>> {
    Ok(workspace
        .read_text(name)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

#[expect(
    clippy::print_stdout,
    reason = "fixture progress is intentionally human-readable"
)]
fn log(message: impl AsRef<str>) {
    println!("[dev] {}", message.as_ref());
}

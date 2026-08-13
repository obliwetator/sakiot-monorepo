//! Remote fixture export and selection widening.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};

use crate::cli::Source;
use crate::config::Config;
use crate::fixtures::remote::{RemoteSql, copy_media, hydrate_remote_media, remote_missing_files};
use crate::fixtures::selection::{
    BulkSelection, CountSelection, NumericResolution, Selector, SelectorKind, resolve_numeric,
};
use crate::fixtures::workspace::{FixtureWorkspace, available_media_files, build_media_lists};
use crate::runner::CommandRunner;

#[derive(Debug)]
pub struct FixtureBundle {
    pub workspace: FixtureWorkspace,
    pub source: Source,
    pub replace_recordings: bool,
    pub guild_ids: Vec<i64>,
    pub summary: FixtureSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureSummary {
    pub recordings: usize,
    pub clips: usize,
    pub stamps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureServerCounts {
    pub recordings: u64,
    pub clips: u64,
    pub stamps: u64,
}

/// Count the records that a bulk sync can select before asking the developer
/// how much media to copy. These are database rows, not a promise that every
/// row still has a physical file; missing media is handled later as a warning.
pub fn fixture_server_counts<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    source: Source,
    ssh: &str,
    guild: i64,
    days: Option<u64>,
) -> Result<FixtureServerCounts> {
    let remote_config = config.remote(source);
    let remote = RemoteSql::configure_read(runner, config, &remote_config, ssh)?;
    let recordings_filter = recordings_filter(Some(guild), days);
    let clips_filter = clips_filter(Some(guild), days);
    let stamps_filter = stamps_filter(Some(guild), days);
    let output = remote.scalar(&format!(
        "SELECT
            (SELECT count(*) FROM audio_files WHERE reaped = false AND end_ts IS NOT NULL {recordings_filter}),
            (SELECT count(*) FROM clips WHERE deleted_at IS NULL AND saved_file_name IS NOT NULL {clips_filter}),
            (SELECT count(*) FROM stamps WHERE true {stamps_filter})"
    ))?;
    parse_fixture_server_counts(&output)
}

fn parse_fixture_server_counts(output: &str) -> Result<FixtureServerCounts> {
    let mut fields = output.trim().split('\t');
    let parse = |field: Option<&str>, name: &str| {
        field
            .ok_or_else(|| anyhow::anyhow!("server count output omitted {name}"))?
            .parse::<u64>()
            .with_context(|| format!("server returned an invalid {name} count"))
    };
    let counts = FixtureServerCounts {
        recordings: parse(fields.next(), "recordings")?,
        clips: parse(fields.next(), "clips")?,
        stamps: parse(fields.next(), "stamps")?,
    };
    if fields.next().is_some() {
        bail!("server count output contained more fields than expected")
    }
    Ok(counts)
}

pub fn export_bundle<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    source: Source,
    ssh: &str,
    bulk: Option<&BulkSelection>,
    selector: Option<&Selector>,
) -> Result<FixtureBundle> {
    let remote_config = config.remote(source);
    let remote = RemoteSql::configure_read(runner, config, &remote_config, ssh)?;
    let workspace = FixtureWorkspace::new()?;
    let bulk_guild = bulk.and_then(|bulk| bulk.guild);
    let selection_recordings_filter =
        recordings_filter(bulk_guild, bulk.and_then(|bulk| bulk.days));
    let selection_clips_filter = clips_filter(bulk_guild, bulk.and_then(|bulk| bulk.days));
    let selection_stamps_filter = stamps_filter(bulk_guild, bulk.and_then(|bulk| bulk.days));

    let (selector, replace_recordings) = if let Some(selector) = selector {
        (Some(resolve_numeric_selector(&remote, selector)?), false)
    } else {
        (None, bulk.is_some_and(|bulk| !bulk.recordings.is_none()))
    };

    let mut clip_ids = Vec::new();
    let mut stamp_ids = Vec::new();
    let mut seed_audio_ids = Vec::new();

    if let Some(selector) = &selector {
        match &selector.kind {
            SelectorKind::Clip(id) => {
                export_tsv(
                    &remote,
                    &workspace,
                    "clip_ids.tsv",
                    &format!(
                        "SELECT clip_id FROM clips WHERE clip_id = {} AND deleted_at IS NULL",
                        sql_literal(&id.to_string())
                    ),
                )?;
                clip_ids = read_strings(&workspace, "clip_ids.tsv")?;
                if clip_ids.is_empty() {
                    bail!(
                        "no live clip in {} ({}) has id {}",
                        remote_config.database,
                        source.label(),
                        id
                    )
                }
            }
            SelectorKind::Stamp(id) => {
                export_tsv(
                    &remote,
                    &workspace,
                    "stamp_ids.tsv",
                    &format!("SELECT id FROM stamps WHERE id = {id}"),
                )?;
                stamp_ids = read_numbers(&workspace, "stamp_ids.tsv")?;
                if stamp_ids.is_empty() {
                    bail!(
                        "no stamp in {} ({}) has id {id}",
                        remote_config.database,
                        source.label()
                    )
                }
            }
            SelectorKind::FileName | SelectorKind::AudioId(_) | SelectorKind::Session(_) => {
                let predicate = selector_predicate(selector)?;
                export_tsv(
                    &remote,
                    &workspace,
                    "audio_file_ids.tsv",
                    &format!("SELECT id FROM audio_files WHERE {predicate} ORDER BY id"),
                )?;
                seed_audio_ids = read_numbers(&workspace, "audio_file_ids.tsv")?;
                if seed_audio_ids.is_empty() {
                    bail!(
                        "no recording in {} ({}) matches {}",
                        remote_config.database,
                        source.label(),
                        selector.value()
                    )
                }
            }
            SelectorKind::Numeric(_) => {
                unreachable!("numeric selectors are resolved before export")
            }
        }
    } else if let Some(bulk) = bulk {
        let recordings_filter = recordings_filter(bulk.guild, bulk.days);
        let clips_filter = clips_filter(bulk.guild, bulk.days);
        let stamps_filter = stamps_filter(bulk.guild, bulk.days);
        if !bulk.recordings.is_none() {
            let query = match bulk.recordings {
                CountSelection::All => format!(
                    "SELECT id FROM audio_files WHERE reaped = false AND end_ts IS NOT NULL {recordings_filter}"
                ),
                CountSelection::None => String::new(),
                CountSelection::Limit(count) => format!(
                    "SELECT id FROM audio_files WHERE reaped = false AND end_ts IS NOT NULL {recordings_filter} ORDER BY id DESC LIMIT {count}"
                ),
            };
            export_tsv(&remote, &workspace, "audio_file_ids.tsv", &query)?;
            seed_audio_ids = read_numbers(&workspace, "audio_file_ids.tsv")?;
            if seed_audio_ids.is_empty() {
                bail!("{} has no matching finalized recordings", source.label())
            }
        } else {
            workspace.write("audio_file_ids.tsv", "")?;
        }
        if !bulk.clips.is_none() {
            let query = match bulk.clips {
                CountSelection::All => format!(
                    "SELECT clip_id FROM clips WHERE deleted_at IS NULL AND saved_file_name IS NOT NULL {clips_filter}"
                ),
                CountSelection::Limit(count) => format!(
                    "SELECT clip_id FROM clips WHERE deleted_at IS NULL AND saved_file_name IS NOT NULL {clips_filter} ORDER BY random() LIMIT {count}"
                ),
                CountSelection::None => String::new(),
            };
            export_tsv(&remote, &workspace, "clip_ids.tsv", &query)?;
            clip_ids = read_strings(&workspace, "clip_ids.tsv")?;
        } else {
            workspace.write("clip_ids.tsv", "")?;
        }
        if !bulk.stamps.is_none() {
            let query = match bulk.stamps {
                CountSelection::All => {
                    format!("SELECT id FROM stamps WHERE true {stamps_filter}")
                }
                CountSelection::Limit(count) => format!(
                    "SELECT id FROM stamps WHERE true {stamps_filter} ORDER BY random() LIMIT {count}"
                ),
                CountSelection::None => String::new(),
            };
            export_tsv(&remote, &workspace, "stamp_ids.tsv", &query)?;
            stamp_ids = read_numbers(&workspace, "stamp_ids.tsv")?;
        } else {
            workspace.write("stamp_ids.tsv", "")?;
        }
    } else {
        bail!("fixture export needs either a bulk selection or a selector")
    }

    let clip_sql = sql_text_list(&clip_ids);
    let stamp_sql = sql_number_list(&stamp_ids);
    let seed_audio_sql = sql_number_list(&seed_audio_ids);
    let mut pulled_sessions = Vec::new();
    if !clip_ids.is_empty() || !stamp_ids.is_empty() {
        let query = format!(
            "SELECT c.recording_session_id FROM clips c
              WHERE c.clip_id IN ({clip_sql}) AND c.recording_session_id IS NOT NULL
             UNION
             SELECT COALESCE(s.recording_session_id, af.recording_session_id)
               FROM stamps s LEFT JOIN audio_files af ON af.id = s.audio_file_id
              WHERE s.id IN ({stamp_sql})
                AND COALESCE(s.recording_session_id, af.recording_session_id) IS NOT NULL"
        );
        export_tsv(&remote, &workspace, "pulled_sessions.tsv", &query)?;
        pulled_sessions = read_numbers(&workspace, "pulled_sessions.tsv")?;
    } else {
        workspace.write("pulled_sessions.tsv", "")?;
    }

    let audio_query = format!(
        "SELECT id FROM audio_files WHERE id IN (
             SELECT id FROM audio_files WHERE id IN ({seed_audio_sql})
             UNION
             SELECT id FROM audio_files WHERE recording_session_id IN ({sessions})
             UNION
             SELECT af.id FROM audio_files af
               JOIN clips c ON c.original_file_name = af.file_name
              WHERE c.clip_id IN ({clip_sql}) AND c.recording_session_id IS NULL
             UNION
             SELECT s.audio_file_id FROM stamps s
              WHERE s.id IN ({stamp_sql}) AND s.audio_file_id IS NOT NULL
         ) {selection_recordings_filter}",
        sessions = sql_number_list(&pulled_sessions),
    );
    export_tsv(&remote, &workspace, "audio_file_ids.tsv", &audio_query)?;
    let audio_ids = read_numbers(&workspace, "audio_file_ids.tsv")?;
    let audio_sql = sql_number_list(&audio_ids);
    export_tsv(
        &remote,
        &workspace,
        "audio_files.tsv",
        &format!("SELECT * FROM audio_files WHERE id IN ({audio_sql}) ORDER BY id DESC"),
    )?;

    let mut session_ids = pulled_sessions;
    let audio_sessions_query = format!(
        "SELECT DISTINCT recording_session_id FROM audio_files WHERE id IN ({audio_sql}) AND recording_session_id IS NOT NULL"
    );
    export_tsv(
        &remote,
        &workspace,
        "audio_session_ids.tsv",
        &audio_sessions_query,
    )?;
    session_ids.extend(read_numbers(&workspace, "audio_session_ids.tsv")?);
    session_ids.sort_unstable();
    session_ids.dedup();
    write_numbers(&workspace, "session_ids.list", &session_ids)?;
    let sessions_sql = sql_number_list(&session_ids);
    export_tsv(
        &remote,
        &workspace,
        "source-sessions.tsv",
        &format!(
            "SELECT file_name, recording_session_id FROM audio_files WHERE id IN ({audio_sql}) AND recording_session_id IS NOT NULL"
        ),
    )?;
    for (file, query) in [
        (
            "recording_sessions.tsv",
            format!("SELECT * FROM recording_sessions WHERE id IN ({sessions_sql})"),
        ),
        (
            "recording_gaps.tsv",
            format!("SELECT * FROM recording_gaps WHERE recording_session_id IN ({sessions_sql})"),
        ),
        (
            "recording_session_events.tsv",
            format!(
                "SELECT * FROM recording_session_events WHERE recording_session_id IN ({sessions_sql})"
            ),
        ),
    ] {
        export_tsv(&remote, &workspace, file, &query)?;
    }

    export_tsv(
        &remote,
        &workspace,
        "clip-ids.list",
        &format!(
            "SELECT clip_id FROM clips
               WHERE deleted_at IS NULL
                 AND (clip_id IN ({clip_sql})
                      OR recording_session_id IN ({sessions_sql})
                      OR (recording_session_id IS NULL AND original_file_name IN
                         (SELECT file_name FROM audio_files WHERE id IN ({audio_sql}))))
                 {selection_clips_filter}"
        ),
    )?;
    clip_ids = read_strings(&workspace, "clip-ids.list")?;
    write_strings(&workspace, "clip-ids.list", &clip_ids)?;
    let all_clip_sql = sql_text_list(&clip_ids);
    export_tsv(
        &remote,
        &workspace,
        "stamp-ids.list",
        &format!(
            "SELECT id FROM stamps WHERE (id IN ({stamp_sql})
               OR recording_session_id IN ({sessions_sql})
               OR audio_file_id IN ({audio_sql}))
               {selection_stamps_filter}"
        ),
    )?;
    stamp_ids = read_numbers(&workspace, "stamp-ids.list")?;
    write_numbers(&workspace, "stamp-ids.list", &stamp_ids)?;
    let all_stamp_sql = sql_number_list(&stamp_ids);

    export_tsv(
        &remote,
        &workspace,
        "clips.tsv",
        &format!("SELECT * FROM clips WHERE clip_id IN ({all_clip_sql})"),
    )?;
    export_tsv(
        &remote,
        &workspace,
        "clip_meta.tsv",
        &format!(
            "SELECT clip_id, guild_id, channel_id, user_id, COALESCE(saved_file_name, '')
               FROM clips WHERE clip_id IN ({all_clip_sql}) ORDER BY created_at, clip_id"
        ),
    )?;
    hydrate_clip_media(
        runner,
        config,
        &remote,
        &remote_config,
        ssh,
        &workspace,
        &all_clip_sql,
    )?;

    export_tsv(
        &remote,
        &workspace,
        "stamps.tsv",
        &format!("SELECT * FROM stamps WHERE id IN ({all_stamp_sql})"),
    )?;
    export_tsv(
        &remote,
        &workspace,
        "stamp_meta.tsv",
        &format!(
            "SELECT id, guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts
               FROM stamps WHERE id IN ({all_stamp_sql}) ORDER BY stamp_ts, id"
        ),
    )?;

    let guild_ids = collect_numeric_field(
        [
            workspace.read_text("audio_files.tsv")?,
            workspace.read_text("clip_meta.tsv")?,
            workspace.read_text("stamp_meta.tsv")?,
        ]
        .iter()
        .map(String::as_str),
        1,
    );
    if guild_ids.is_empty() {
        bail!("the selection names no guild; nothing importable")
    }
    let mut user_ids = collect_user_ids(&workspace)?;
    let span = event_span(&config.event_margin, &audio_sql, &sessions_sql);
    let guilds = sql_number_list(&guild_ids);
    export_tsv(
        &remote,
        &workspace,
        "voice_state_event_types.tsv",
        "SELECT * FROM voice_state_event_types",
    )?;
    export_tsv(
        &remote,
        &workspace,
        "voice_state_events.tsv",
        &format!(
            "WITH span AS ({span})
             SELECT v.* FROM voice_state_events v, span
              WHERE v.guild_id IN ({guilds})
                AND v.occurred_at >= span.lo AND v.occurred_at <= span.hi
              ORDER BY v.occurred_at, v.id"
        ),
    )?;
    let has_connection = !remote.table_columns("voice_connection_events")?.is_empty();
    if has_connection {
        export_tsv(
            &remote,
            &workspace,
            "voice_connection_events.tsv",
            &format!(
                "WITH span AS ({span})
                 SELECT e.* FROM voice_connection_events e, span
                  WHERE e.guild_id IN ({guilds})
                    AND e.completed_at >= span.lo AND e.started_at <= span.hi
                  ORDER BY e.completed_at, e.id"
            ),
        )?;
    } else {
        workspace.write("voice_connection_events.tsv", "")?;
    }
    user_ids.extend(
        workspace
            .read_text("voice_state_events.tsv")?
            .lines()
            .filter_map(|line| line.split('\t').nth(3)?.parse::<i64>().ok()),
    );
    user_ids.sort_unstable();
    user_ids.dedup();
    let user_sql = sql_number_list(&user_ids);
    let guild_sql = sql_number_list(&guild_ids);
    for (file, query) in [
        (
            "guilds.tsv",
            format!("SELECT * FROM guilds WHERE id IN ({guild_sql})"),
        ),
        (
            "roles.tsv",
            format!("SELECT * FROM roles WHERE guild_id IN ({guild_sql})"),
        ),
        (
            "user_roles.tsv",
            format!(
                "SELECT ur.user_id, ur.role_id FROM user_roles ur JOIN roles r ON r.role_id = ur.role_id WHERE r.guild_id IN ({guild_sql})"
            ),
        ),
        (
            "channels.tsv",
            format!("SELECT * FROM channels WHERE guild_id IN ({guild_sql})"),
        ),
        (
            "channel_permissions.tsv",
            format!(
                "SELECT cp.* FROM channel_permissions cp JOIN channels c ON c.channel_id = cp.channel_id WHERE c.guild_id IN ({guild_sql})"
            ),
        ),
        (
            "user_names.tsv",
            format!("SELECT * FROM user_names WHERE user_id IN ({user_sql})"),
        ),
        (
            "user_nicknames.tsv",
            format!(
                "SELECT * FROM user_nicknames WHERE user_id IN ({user_sql}) AND guild_id IN ({guild_sql})"
            ),
        ),
        (
            "user_name_history.tsv",
            format!("SELECT * FROM user_name_history WHERE user_id IN ({user_sql})"),
        ),
    ] {
        export_tsv(&remote, &workspace, file, &query)?;
    }
    for (file, query) in [
        (
            "audio_file_finalize_reasons.tsv",
            "SELECT * FROM audio_file_finalize_reasons".to_string(),
        ),
        (
            "user_name_event_types.tsv",
            "SELECT * FROM user_name_event_types".to_string(),
        ),
        ("channel_type.tsv", "SELECT * FROM channel_type".to_string()),
    ] {
        export_tsv(&remote, &workspace, file, &query)?;
    }

    build_media_lists(&workspace)?;
    hydrate_recording_media(
        runner,
        config,
        &remote,
        &remote_config,
        ssh,
        &workspace,
        &audio_ids,
    )?;
    copy_media(
        runner,
        ssh,
        &remote_config.data_dir,
        &workspace.path("files.list"),
        &workspace.media,
    )?;
    let available = available_media_files(&workspace)?;
    workspace.write(
        "new-files.list",
        if available.is_empty() {
            String::new()
        } else {
            format!("{}\n", available.join("\n"))
        },
    )?;
    let requested_clips = workspace.read_text("clip-media.list").unwrap_or_default();
    let missing_clips = requested_clips
        .lines()
        .filter(|file| !available.iter().any(|present| present == file))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !missing_clips.is_empty() {
        log(format!(
            "note: {} clip(s) imported without audio",
            missing_clips.len()
        ));
    }
    let files_list = workspace.read_text("files.list")?;
    let requested_recordings = files_list
        .lines()
        .filter(|file| file.starts_with("voice_recordings/"))
        .collect::<Vec<_>>();
    if requested_recordings
        .iter()
        .any(|file| !available.iter().any(|present| present == file))
    {
        let missing = requested_recordings
            .iter()
            .filter(|file| !available.iter().any(|present| present == **file))
            .count();
        log(format!(
            "warning: {missing} selected recording media file(s) were unavailable from {}; continuing without them",
            source.label()
        ));
    }

    let summary = FixtureSummary {
        recordings: workspace
            .read_text("audio_files.tsv")?
            .lines()
            .filter(|line| !line.is_empty())
            .count(),
        clips: workspace
            .read_text("clips.tsv")?
            .lines()
            .filter(|line| !line.is_empty())
            .count(),
        stamps: workspace
            .read_text("stamps.tsv")?
            .lines()
            .filter(|line| !line.is_empty())
            .count(),
    };
    Ok(FixtureBundle {
        workspace,
        source,
        replace_recordings,
        guild_ids,
        summary,
    })
}

fn resolve_numeric_selector<R: CommandRunner + ?Sized>(
    remote: &RemoteSql<'_, R>,
    selector: &Selector,
) -> Result<Selector> {
    let SelectorKind::Numeric(id) = selector.kind else {
        return Ok(selector.clone());
    };
    let audio_exists = !remote
        .scalar(&format!(
            "SELECT 1 FROM audio_files WHERE id = {id} LIMIT 1"
        ))?
        .is_empty();
    let session_exists = !remote
        .scalar(&format!(
            "SELECT 1 FROM recording_sessions WHERE id = {id} LIMIT 1"
        ))?
        .is_empty();
    let kind = match resolve_numeric(audio_exists, session_exists) {
        NumericResolution::Audio => SelectorKind::AudioId(id),
        NumericResolution::Session => SelectorKind::Session(id),
        NumericResolution::Ambiguous => bail!(
            "{id} is both an audio_files.id and recording_sessions.id; use --kind recording or --kind session"
        ),
        NumericResolution::Missing => bail!("no recording or session has source id {id}"),
    };
    Ok(Selector {
        input: selector.input.clone(),
        kind,
        host: selector.host.clone(),
    })
}

fn hydrate_recording_media<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    remote: &RemoteSql<'_, R>,
    source: &crate::config::RemoteConfig,
    ssh: &str,
    workspace: &FixtureWorkspace,
    audio_ids: &[i64],
) -> Result<()> {
    if audio_ids.is_empty() {
        return Ok(());
    }
    let recording_files = workspace
        .read_text("files.list")?
        .lines()
        .filter(|file| file.starts_with("voice_recordings/"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let missing = remote_missing_files(remote, &source.data_dir, &recording_files)?;
    if missing.is_empty() {
        return Ok(());
    }
    let ids = audio_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if let Err(error) = hydrate_remote_media(
        runner,
        config,
        source,
        ssh,
        "recordings",
        "--audio-file-ids",
        &ids,
    ) {
        log(format!(
            "warning: could not restore {} missing recording media file(s); importing metadata without unavailable audio: {error}",
            missing.len()
        ));
    }
    Ok(())
}

fn selector_predicate(selector: &Selector) -> Result<String> {
    Ok(match &selector.kind {
        SelectorKind::FileName => format!("file_name = {}", sql_literal(&selector.value())),
        SelectorKind::AudioId(id) => format!("id = {id}"),
        SelectorKind::Session(id) => format!("recording_session_id = {id}"),
        other => bail!("selector kind {other:?} cannot select audio files"),
    })
}

fn hydrate_clip_media<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    remote: &RemoteSql<'_, R>,
    source: &crate::config::RemoteConfig,
    ssh: &str,
    workspace: &FixtureWorkspace,
    clip_sql: &str,
) -> Result<()> {
    let meta = workspace.read_text("clip_meta.tsv")?;
    let wanted = meta
        .lines()
        .filter_map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            (fields.len() >= 5 && !fields[4].is_empty())
                .then(|| format!("clips/{}\t{}", fields[4], fields[0]))
        })
        .collect::<Vec<_>>();
    workspace.write(
        "clip-media.tsv",
        if wanted.is_empty() {
            String::new()
        } else {
            format!("{}\n", wanted.join("\n"))
        },
    )?;
    let files = wanted
        .iter()
        .filter_map(|line| line.split('\t').next().map(str::to_string))
        .collect::<Vec<_>>();
    let missing = remote_missing_files(remote, &source.data_dir, &files)?;
    if !missing.is_empty() {
        let missing_set = missing.into_iter().collect::<BTreeSet<_>>();
        let ids = wanted
            .iter()
            .filter_map(|line| {
                let mut parts = line.split('\t');
                let file = parts.next()?;
                let id = parts.next()?;
                missing_set.contains(file).then_some(id)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(",");
        if let Err(error) =
            hydrate_remote_media(runner, config, source, ssh, "clips", "--clip-ids", &ids)
        {
            log(format!(
                "warning: could not restore {} clip archive file(s); importing their metadata without audio: {error}",
                missing_set.len()
            ));
        }
    }
    let _ = clip_sql;
    Ok(())
}

fn export_tsv<R: CommandRunner + ?Sized>(
    remote: &RemoteSql<'_, R>,
    workspace: &FixtureWorkspace,
    name: &str,
    query: &str,
) -> Result<()> {
    if query.is_empty() {
        workspace.write(name, "")?;
        return Ok(());
    }
    workspace.write(name, remote.copy_out(query)?).map(|_| ())
}

fn read_strings(workspace: &FixtureWorkspace, name: &str) -> Result<Vec<String>> {
    Ok(workspace
        .read_text(name)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn read_numbers(workspace: &FixtureWorkspace, name: &str) -> Result<Vec<i64>> {
    let mut values = workspace
        .read_text(name)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.parse::<i64>()
                .with_context(|| format!("{line} in {name} is not an integer"))
        })
        .collect::<Result<Vec<_>>>()?;
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

fn write_numbers(workspace: &FixtureWorkspace, name: &str, values: &[i64]) -> Result<()> {
    workspace.write(
        name,
        if values.is_empty() {
            String::new()
        } else {
            format!(
                "{}\n",
                values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        },
    )?;
    Ok(())
}

fn write_strings(workspace: &FixtureWorkspace, name: &str, values: &[String]) -> Result<()> {
    workspace.write(
        name,
        if values.is_empty() {
            String::new()
        } else {
            format!("{}\n", values.join("\n"))
        },
    )?;
    Ok(())
}

fn collect_numeric_field<'a>(sources: impl IntoIterator<Item = &'a str>, index: usize) -> Vec<i64> {
    let mut values = sources
        .into_iter()
        .flat_map(|source| source.lines())
        .filter_map(|line| line.split('\t').nth(index)?.parse::<i64>().ok())
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn collect_user_ids(workspace: &FixtureWorkspace) -> Result<Vec<i64>> {
    let mut values = collect_numeric_field(
        [
            workspace.read_text("audio_files.tsv")?,
            workspace.read_text("clip_meta.tsv")?,
            workspace.read_text("stamp_meta.tsv")?,
        ]
        .iter()
        .map(String::as_str),
        3,
    );
    values.extend(collect_numeric_field(
        [workspace.read_text("stamp_meta.tsv")?]
            .iter()
            .map(String::as_str),
        4,
    ));
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

fn guild_filter(guild: Option<i64>) -> String {
    guild.map_or_else(String::new, |guild| format!("AND guild_id = {guild}"))
}

fn recordings_filter(guild: Option<i64>, days: Option<u64>) -> String {
    recent_filter(
        guild_filter(guild),
        "to_timestamp(end_ts::double precision / 1000.0)",
        days,
    )
}

fn clips_filter(guild: Option<i64>, days: Option<u64>) -> String {
    recent_filter(guild_filter(guild), "created_at", days)
}

fn stamps_filter(guild: Option<i64>, days: Option<u64>) -> String {
    recent_filter(
        guild_filter(guild),
        "to_timestamp(stamp_ts::double precision / 1000.0)",
        days,
    )
}

fn recent_filter(mut filter: String, timestamp: &str, days: Option<u64>) -> String {
    if let Some(days) = days {
        filter.push_str(&format!(
            " AND {timestamp} >= CURRENT_TIMESTAMP - ({days} * INTERVAL '1 day')"
        ));
    }
    filter
}

fn event_span(margin: &str, audio_sql: &str, sessions_sql: &str) -> String {
    format!(
        "SELECT MIN(lo) - interval {margin} AS lo, MAX(hi) + interval {margin} AS hi FROM (
           SELECT to_timestamp(COALESCE(af.start_ts, 0) / 1000.0),
                  to_timestamp(COALESCE(af.end_ts, af.start_ts, 0) / 1000.0)
             FROM audio_files af WHERE af.id IN ({audio_sql})
           UNION ALL
           SELECT rs.started_at, COALESCE(rs.ended_at, rs.started_at)
             FROM recording_sessions rs WHERE rs.id IN ({sessions_sql})
         ) s(lo, hi)",
        margin = sql_literal(margin)
    )
}

fn sql_number_list(values: &[i64]) -> String {
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

fn sql_text_list(values: &[String]) -> String {
    if values.is_empty() {
        "''".into()
    } else {
        values
            .iter()
            .map(|value| sql_literal(value))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[expect(
    clippy::print_stdout,
    reason = "fixture progress is intentionally human-readable"
)]
fn log(message: String) {
    println!("[dev] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_counts() {
        assert_eq!(
            parse_fixture_server_counts("509\t36\t123\n").unwrap(),
            FixtureServerCounts {
                recordings: 509,
                clips: 36,
                stamps: 123,
            }
        );
    }

    #[test]
    fn rejects_malformed_server_counts() {
        assert!(parse_fixture_server_counts("509\t36").is_err());
        assert!(parse_fixture_server_counts("509\t36\t123\t4").is_err());
        assert!(parse_fixture_server_counts("many\t36\t123").is_err());
    }

    #[test]
    fn global_days_filter_uses_each_source_timestamp() {
        let recordings = recordings_filter(Some(42), Some(7));
        let clips = clips_filter(Some(42), Some(7));
        let stamps = stamps_filter(Some(42), Some(7));

        assert!(recordings.contains("guild_id = 42"));
        assert!(recordings.contains("to_timestamp(end_ts::double precision / 1000.0)"));
        assert!(clips.contains("created_at >= CURRENT_TIMESTAMP - (7 * INTERVAL '1 day')"));
        assert!(stamps.contains("to_timestamp(stamp_ts::double precision / 1000.0)"));
        assert!(recordings.contains("CURRENT_TIMESTAMP"));
        assert!(stamps.contains("CURRENT_TIMESTAMP"));
    }
}

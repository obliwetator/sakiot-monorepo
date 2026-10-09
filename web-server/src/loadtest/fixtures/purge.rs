//! Deleting a fixture guild's rows and the media files it linked.

use super::*;

#[derive(Serialize, Default)]
pub(super) struct TeardownReport {
    guild_id: String,
    rows: u64,
    files: u64,
}

pub(super) async fn teardown(
    state: &LoadtestState,
    fixture: u32,
) -> Result<TeardownReport, AppError> {
    let guild_id = ids::guild_id(fixture);
    let (low, high) = ids::slot(guild_id);
    state.sims.stop_guild(guild_id).await;

    // Collect what names the derived files before the rows go.
    let stems: Vec<String> =
        sqlx::query_scalar("SELECT file_name FROM audio_files WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let session_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM recording_sessions WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let clip_rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT clip_id, saved_file_name FROM clips WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let media_jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM media_jobs WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;
    let composition_jobs: Vec<String> =
        sqlx::query_scalar("SELECT id FROM composition_jobs WHERE guild_id = $1")
            .bind(guild_id)
            .fetch_all(&state.pool)
            .await?;

    let mut tx = state.pool.begin().await?;
    let mut rows = 0;
    let statements: [&str; 26] = [
        "DELETE FROM media_objects WHERE audio_file_id IN (SELECT id FROM audio_files WHERE guild_id = $1)
            OR clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)",
        "DELETE FROM stamps WHERE guild_id = $1",
        "DELETE FROM clip_source_history WHERE target_clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)
            OR source_clip_id IN (SELECT clip_id FROM clips WHERE guild_id = $1)",
        "DELETE FROM clips WHERE guild_id = $1",
        "DELETE FROM media_jobs WHERE guild_id = $1",
        "DELETE FROM composition_jobs WHERE guild_id = $1",
        "DELETE FROM recording_deletion_jobs WHERE guild_id = $1",
        "DELETE FROM audio_files WHERE guild_id = $1",
        "DELETE FROM recording_sessions WHERE guild_id = $1",
        "DELETE FROM voice_state_events WHERE guild_id = $1",
        "DELETE FROM voice_events WHERE guild_id = $1",
        "DELETE FROM voice_connection_events WHERE guild_id = $1",
        "DELETE FROM voice_session_leases WHERE guild_id = $1",
        "DELETE FROM jam_invocations WHERE guild_id = $1",
        "DELETE FROM user_jam_cooldown_overrides WHERE guild_id = $1",
        "DELETE FROM guild_jam_cooldowns WHERE guild_id = $1",
        "DELETE FROM guild_voice_settings WHERE guild_id = $1",
        "DELETE FROM guild_recording_policy WHERE guild_id = $1",
        "DELETE FROM recording_opt_outs WHERE guild_id = $1",
        "DELETE FROM user_guilds WHERE id = $1",
        "DELETE FROM user_nicknames WHERE guild_id = $1",
        "DELETE FROM user_name_history WHERE guild_id = $1",
        "DELETE FROM user_roles WHERE role_id IN (SELECT role_id FROM roles WHERE guild_id = $1)",
        // Cascades to guild_members and voice_presence.
        "DELETE FROM guild_projection_state WHERE guild_id = $1",
        "DELETE FROM guilds_present WHERE guild_id = $1",
        // Cascades to roles, channels and channel_permissions.
        "DELETE FROM guilds WHERE id = $1",
    ];
    for statement in statements {
        rows += sqlx::query(statement)
            .bind(guild_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    for statement in [
        "DELETE FROM user_name_history WHERE user_id >= $1 AND user_id < $2",
        "DELETE FROM user_names WHERE user_id >= $1 AND user_id < $2",
        "DELETE FROM discord_auth_user WHERE id >= $1 AND id < $2",
    ] {
        rows += sqlx::query(statement)
            .bind(low)
            .bind(high)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;

    let files = purge_files(
        guild_id,
        &stems,
        &session_ids,
        &clip_rows,
        &media_jobs,
        &composition_jobs,
    )
    .await?;
    tracing::info!(fixture, guild_id, rows, files, "load-test fixture deleted");
    Ok(TeardownReport {
        guild_id: guild_id.to_string(),
        rows,
        files,
    })
}

/// Remove every file the fixture and the load against it produced: the
/// guild's recording and silence-free trees (originals, live HLS and mix
/// caches), waveforms, logical-session renders, clips, and job outputs.
async fn purge_files(
    guild_id: i64,
    stems: &[String],
    session_ids: &[i64],
    clips: &[(String, Option<String>)],
    media_jobs: &[String],
    composition_jobs: &[String],
) -> Result<u64, AppError> {
    let roots = sakiot_paths::DataRoots::from_env();
    let guild = guild_id.to_string();
    let stems: HashSet<String> = stems.iter().cloned().collect();
    let session_prefixes: Vec<String> = session_ids
        .iter()
        .flat_map(|id| {
            [
                format!("logical-session-{id}-"),
                format!("logical-session-{id}."),
            ]
        })
        .collect();
    let clip_prefixes: Vec<String> = clips.iter().map(|(id, _)| format!("clip-{id}-")).collect();
    let saved_clips: Vec<String> = clips
        .iter()
        .filter_map(|(_, saved)| saved.clone())
        .collect();
    let jobs: Vec<String> = media_jobs.iter().chain(composition_jobs).cloned().collect();
    let session_renders: Vec<String> = session_ids.iter().map(|id| format!("{id}-")).collect();

    tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        let mut removed = 0;
        for tree in [roots.recordings.join(&guild), roots.no_silence.join(&guild)] {
            if tree.exists() {
                removed += count_files(&tree);
                std::fs::remove_dir_all(&tree)?;
            }
        }
        removed += remove_matching(&roots.waveforms, |name| {
            let stem = name
                .strip_prefix(sakiot_paths::NO_SILENCE_PREFIX)
                .unwrap_or(name);
            stem.split_once('.')
                .is_some_and(|(stem, _)| stems.contains(stem))
                || session_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                || clip_prefixes.iter().any(|prefix| name.starts_with(prefix))
        })?;
        removed += remove_matching(&roots.no_silence.join("logical_sessions"), |name| {
            session_renders
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })?;
        for saved in &saved_clips {
            if let Some(path) = sakiot_paths::safe_join(&roots.clips, Path::new(saved))
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        for dir in [
            roots.recordings.join(".media-jobs"),
            roots.recordings.join(".composition-jobs"),
            roots.clips.join("compositions"),
        ] {
            removed += remove_matching(&dir, |name| {
                jobs.iter().any(|job| name.starts_with(job.as_str()))
            })?;
        }
        Ok(removed)
    })
    .await
    .map_err(|_| AppError::InternalError)?
    .map_err(AppError::from)
}

fn remove_matching(dir: &Path, matches: impl Fn(&str) -> bool) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if !matches(&name.to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            removed += count_files(&path);
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn count_files(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| match entry.file_type() {
                    Ok(kind) if kind.is_dir() => count_files(&entry.path()),
                    _ => 1,
                })
                .sum()
        })
        .unwrap_or(0)
}

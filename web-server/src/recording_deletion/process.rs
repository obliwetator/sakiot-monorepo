//! What one deletion attempt removes: a session's fragments, or an erased
//! clip, with the clips and jobs that depend on them and their archived
//! sources.

use super::*;

pub(super) struct Fragment {
    pub(super) id: i64,
    pub(super) guild_id: i64,
    pub(super) channel_id: i64,
    pub(super) year: i32,
    pub(super) month: i32,
    pub(super) file_name: String,
}
pub(super) struct Clip {
    id: String,
    saved: Option<String>,
}
struct MediaJob {
    id: String,
}
struct CompositionJob {
    id: String,
}

pub(super) async fn process_with_roots(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    roots: &DataRoots,
) -> Result<(), AttemptError> {
    // Do not race a renderer already admitted before the tombstone. Queued
    // attempts recheck source access and will fail once they start.
    if active_guild_media_jobs(pool, job.guild_id).await? > 0 {
        return Err(AttemptError::Waiting);
    }
    match &job.subject {
        Subject::Session(session_id) => process_session(pool, media, job, *session_id, roots).await,
        Subject::Clip(clip_id) => process_clip(pool, media, job, clip_id, roots).await,
    }
}

async fn process_session(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    session_id: i64,
    roots: &DataRoots,
) -> Result<(), AttemptError> {
    let session = sqlx::query!(
        "SELECT starting_channel_id,started_at,ended_at,deletion_requested_at FROM recording_sessions WHERE id=$1 AND guild_id=$2",
        session_id,
        job.guild_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::FileNotFound)?;
    if session.deletion_requested_at.is_none() {
        return Err(AttemptError::failed(
            ErrorKind::InternalError,
            "recording is not tombstoned",
        ));
    }
    let (started, ended, starting_channel) = (
        session.started_at,
        session.ended_at,
        session.starting_channel_id,
    );
    let fragments = session_fragments(pool, session_id).await?;
    let clips = {
        let mut connection = pool.acquire().await?;
        related_clips(&mut connection, job.guild_id, session_id, &fragments).await?
    };
    require_safe_clip_ids(&clips)?;
    let media_jobs = related_media_jobs(pool, job.guild_id, session_id).await?;
    let composition_jobs = related_composition_jobs(pool, job, &clips).await?;
    stage(pool, job, "purging_archive").await?;
    // Archive uploads hold this advisory lock for their entire transfer. A
    // claimed upload that starts afterwards sees the tombstone and skips.
    for fragment in &fragments {
        let object_id = sqlx::query_scalar!(
            "SELECT id FROM media_objects WHERE audio_file_id=$1",
            fragment.id
        )
        .fetch_optional(pool)
        .await?;
        purge_source(
            pool,
            media,
            object_id,
            &format!("media/v1/recordings/{}/", fragment.id),
            job,
        )
        .await?;
    }
    purge_clip_archive(pool, media, job, &clips).await?;
    stage(pool, job, "purging_local").await?;
    for fragment in &fragments {
        purge_fragment(roots, fragment).await?;
    }
    purge_session_cache(
        roots,
        job.guild_id,
        session_id,
        starting_channel,
        started,
        ended,
        &fragments,
    )
    .await?;
    purge_clip_local(roots, &clips, &composition_jobs).await?;
    for media_job in &media_jobs {
        purge_job_outputs(&roots.recordings.join(".media-jobs"), &media_job.id).await?;
    }
    stage(pool, job, "removing_records").await?;
    let mut tx = removal_transaction(pool, job).await?;
    sqlx::query!(
        "DELETE FROM media_objects WHERE audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1)",
        session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM stamps WHERE recording_session_id=$1 OR audio_file_id IN (SELECT id FROM audio_files WHERE recording_session_id=$1)",
        session_id
    )
    .execute(&mut *tx)
    .await?;
    delete_clip_rows(&mut tx, &clips, &composition_jobs).await?;
    sqlx::query!(
        "DELETE FROM media_jobs WHERE id=ANY($1)",
        &media_jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM audio_files WHERE recording_session_id=$1",
        session_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM recording_sessions WHERE id=$1 AND deletion_requested_at IS NOT NULL",
        session_id
    )
    .execute(&mut *tx)
    .await?;
    finish(tx, job).await
}

/// A clip a speaker erased: its archived versions, local file and cached
/// peaks, with every clip composed from it, then the rows. A clip already
/// gone went with its session's purge, which took those clips too.
async fn process_clip(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    clip_id: &str,
    roots: &DataRoots,
) -> Result<(), AttemptError> {
    let deleted = sqlx::query_scalar!(
        r#"SELECT deleted_at IS NOT NULL AS "deleted!" FROM clips WHERE guild_id=$1 AND clip_id=$2"#,
        job.guild_id,
        clip_id
    )
    .fetch_optional(pool)
    .await?;
    let clips = match deleted {
        None => Vec::new(),
        Some(false) => {
            return Err(AttemptError::failed(
                ErrorKind::InternalError,
                "clip is not tombstoned",
            ));
        }
        Some(true) => {
            let mut connection = pool.acquire().await?;
            clip_with_dependents(&mut connection, job.guild_id, clip_id).await?
        }
    };
    require_safe_clip_ids(&clips)?;
    let composition_jobs = related_composition_jobs(pool, job, &clips).await?;
    stage(pool, job, "purging_archive").await?;
    purge_clip_archive(pool, media, job, &clips).await?;
    stage(pool, job, "purging_local").await?;
    purge_clip_local(roots, &clips, &composition_jobs).await?;
    stage(pool, job, "removing_records").await?;
    let mut tx = removal_transaction(pool, job).await?;
    delete_clip_rows(&mut tx, &clips, &composition_jobs).await?;
    finish(tx, job).await
}

/// Clip ids name archive prefixes and local files.
fn require_safe_clip_ids(clips: &[Clip]) -> Result<(), AttemptError> {
    for clip in clips {
        if clip.id.is_empty()
            || !clip
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AttemptError::failed(
                ErrorKind::UnsafeMediaReference,
                format!("clip {:?} has an unsafe archive identifier", clip.id),
            ));
        }
    }
    Ok(())
}

/// Every archived version of the clips.
async fn purge_clip_archive(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    job: &Claimed,
    clips: &[Clip],
) -> Result<(), AttemptError> {
    for clip in clips {
        let object_id =
            sqlx::query_scalar!("SELECT id FROM media_objects WHERE clip_id=$1", clip.id)
                .fetch_optional(pool)
                .await?;
        if let Some(object_id) = object_id {
            purge_source(
                pool,
                media,
                Some(object_id),
                &format!("media/v1/clips/{}/", clip.id),
                job,
            )
            .await?;
        } else if let Some(archive) = media.archive() {
            // Reconciliation may not yet have tracked a short-lived clip.
            archive
                .purge_versions(&format!("media/v1/clips/{}/", clip.id))
                .await
                .map_err(|error| AppError::MediaArchiveUnavailable(error.to_string()))?;
        }
    }
    Ok(())
}

/// The clips' local files and cached peaks, and whatever the composition
/// jobs that made them left behind.
async fn purge_clip_local(
    roots: &DataRoots,
    clips: &[Clip],
    composition_jobs: &[CompositionJob],
) -> Result<(), AttemptError> {
    for clip in clips {
        if let Some(saved) = &clip.saved {
            remove_file_if_exists(&crate::media_archive::clip_local_path_in(
                &roots.clips,
                saved,
            )?)
            .await?;
        }
        purge_clip_waveforms(&roots.waveforms, &clip.id).await?;
    }
    for composition_job in composition_jobs {
        uuid::Uuid::parse_str(&composition_job.id).map_err(|_| {
            AttemptError::failed(
                ErrorKind::UnsafeMediaReference,
                format!(
                    "composition job {:?} has an invalid identifier",
                    composition_job.id
                ),
            )
        })?;
        remove_dir_if_exists(
            &roots
                .clips
                .join(".composition-jobs")
                .join(&composition_job.id),
        )
        .await?;
        purge_job_outputs(&roots.clips.join("compositions"), &composition_job.id).await?;
    }
    Ok(())
}

/// The transaction that removes the records, once the attempt still owns
/// the job.
async fn removal_transaction<'p>(
    pool: &'p Pool<Postgres>,
    job: &Claimed,
) -> Result<sqlx::Transaction<'p, Postgres>, AttemptError> {
    let mut tx = pool.begin().await?;
    let owned = sqlx::query_scalar!(
        "SELECT id FROM recording_deletion_jobs WHERE id=$1 AND attempt_token=$2 AND state='running' AND lease_expires_at>now() FOR UPDATE",
        job.id,
        job.token
    )
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(AttemptError::LeaseLost);
    }
    // A new media request may have entered after the initial check. Leave the
    // tombstone in place and retry rather than publish/erase across it.
    if active_guild_media_jobs(&mut *tx, job.guild_id).await? > 0 {
        return Err(AttemptError::Waiting);
    }
    Ok(tx)
}

/// The clips' rows, and the composition jobs and source history naming them.
async fn delete_clip_rows(
    tx: &mut sqlx::PgConnection,
    clips: &[Clip],
    composition_jobs: &[CompositionJob],
) -> Result<(), sqlx::Error> {
    let clip_ids = clip_ids(clips);
    sqlx::query!("DELETE FROM media_objects WHERE clip_id=ANY($1)", &clip_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM clips WHERE clip_id=ANY($1)", &clip_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "DELETE FROM composition_jobs WHERE id=ANY($1)",
        &composition_jobs
            .iter()
            .map(|j| j.id.clone())
            .collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM clip_source_history WHERE source_clip_id=ANY($1) OR target_clip_id=ANY($1)",
        &clip_ids
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Marks the job done and commits its removals.
async fn finish(
    mut tx: sqlx::Transaction<'_, Postgres>,
    job: &Claimed,
) -> Result<(), AttemptError> {
    sqlx::query!(
        "UPDATE recording_deletion_jobs SET state='ready',stage='ready',attempt_token=NULL,lease_expires_at=NULL,error=NULL,error_kind=NULL,finished_at=now(),updated_at=now() WHERE id=$1 AND attempt_token=$2",
        job.id,
        job.token
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn purge_source(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    object_id: Option<i64>,
    prefix: &str,
    job: &Claimed,
) -> Result<(), AttemptError> {
    let mut lock = pool.begin().await?;
    if let Some(object_id) = object_id {
        sqlx::query!(
            "SELECT pg_advisory_xact_lock($1,$2)",
            ARCHIVE_LOCK_NAMESPACE,
            object_id as i32
        )
        .execute(&mut *lock)
        .await?;
    }
    stage(pool, job, "purging_archive").await?;
    if let Some(archive) = media.archive() {
        archive
            .purge_versions(prefix)
            .await
            .map_err(|error| AppError::MediaArchiveUnavailable(error.to_string()))?;
    }
    lock.commit().await?;
    Ok(())
}

/// The session's physical fragments, in upload order.
pub(super) async fn session_fragments(
    executor: impl sqlx::PgExecutor<'_>,
    session_id: i64,
) -> Result<Vec<Fragment>, sqlx::Error> {
    sqlx::query_as!(
        Fragment,
        "SELECT id,guild_id,channel_id,year,month,file_name FROM audio_files WHERE recording_session_id=$1 ORDER BY id",
        session_id
    )
    .fetch_all(executor)
    .await
}

/// Queued or running media and composition jobs in the guild.
async fn active_guild_media_jobs(
    executor: impl sqlx::PgExecutor<'_>,
    guild_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM media_jobs WHERE guild_id=$1 AND state IN ('queued','running')) + (SELECT count(*) FROM composition_jobs WHERE guild_id=$1 AND state IN ('queued','running')) AS "active!""#,
        guild_id
    )
    .fetch_one(executor)
    .await
}

pub(super) fn clip_ids(clips: &[Clip]) -> Vec<String> {
    clips.iter().map(|clip| clip.id.clone()).collect()
}

/// A guild clip as the dependency scan needs it.
struct GuildClip {
    clip_id: String,
    saved_file_name: Option<String>,
    recording_session_id: Option<i64>,
    original_file_name: Option<String>,
    channel_id: Option<i64>,
    composition: Option<serde_json::Value>,
}

async fn guild_clips(
    connection: &mut sqlx::PgConnection,
    guild_id: i64,
) -> Result<Vec<GuildClip>, sqlx::Error> {
    sqlx::query_as!(
        GuildClip,
        "SELECT clip_id,saved_file_name,recording_session_id,original_file_name,channel_id,composition FROM clips WHERE guild_id=$1",
        guild_id
    )
    .fetch_all(&mut *connection)
    .await
}

/// The session's clips (cut from it, or from one of its fragments) and every
/// clip composed from them.
pub(super) async fn related_clips(
    connection: &mut sqlx::PgConnection,
    guild_id: i64,
    session_id: i64,
    fragments: &[Fragment],
) -> Result<Vec<Clip>, AppError> {
    let rows = guild_clips(connection, guild_id).await?;
    let sources = rows.iter().filter(|row| {
        row.recording_session_id == Some(session_id)
            || fragments.iter().any(|f| {
                Some(f.channel_id) == row.channel_id
                    && row.original_file_name.as_deref() == Some(f.file_name.as_str())
            })
    });
    with_dependents(connection, &rows, sources).await
}

/// The clip and every clip composed from it, or nothing if it is gone.
pub(super) async fn clip_with_dependents(
    connection: &mut sqlx::PgConnection,
    guild_id: i64,
    clip_id: &str,
) -> Result<Vec<Clip>, AppError> {
    let rows = guild_clips(connection, guild_id).await?;
    let sources = rows.iter().filter(|row| row.clip_id == clip_id);
    with_dependents(connection, &rows, sources).await
}

/// `sources`, then every clip of `rows` composed from one of them, now or in
/// an earlier revision.
async fn with_dependents<'r>(
    connection: &mut sqlx::PgConnection,
    rows: &'r [GuildClip],
    sources: impl Iterator<Item = &'r GuildClip>,
) -> Result<Vec<Clip>, AppError> {
    let mut selected = HashSet::new();
    let mut clips = Vec::new();
    for row in sources {
        selected.insert(row.clip_id.clone());
        clips.push(Clip {
            id: row.clip_id.clone(),
            saved: row.saved_file_name.clone(),
        });
    }
    // A composition can depend on any session clip or physical fragment clip.
    // Composed clips cannot themselves be sources, so one pass suffices.
    for row in rows {
        if selected.contains(&row.clip_id) {
            continue;
        }
        let dependent = row
            .composition
            .as_ref()
            .and_then(|c| c.get("segments"))
            .and_then(|s| s.as_array())
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    segment
                        .get("source_id")
                        .and_then(|id| id.as_str())
                        .is_some_and(|id| selected.contains(id))
                })
            });
        if dependent {
            selected.insert(row.clip_id.clone());
            clips.push(Clip {
                id: row.clip_id.clone(),
                saved: row.saved_file_name.clone(),
            });
        }
    }
    // This ledger survives composition-job cleanup and retains dependencies
    // even after a clip was overwritten with an unrelated composition.
    let sources: Vec<String> = selected.iter().cloned().collect();
    let historical_targets = sqlx::query_scalar!(
        r#"SELECT DISTINCT target_clip_id AS "target_clip_id!" FROM clip_source_history WHERE source_clip_id=ANY($1)"#,
        &sources
    )
    .fetch_all(&mut *connection)
    .await?;
    for target in historical_targets {
        if selected.contains(&target) {
            continue;
        }
        if let Some(row) = rows.iter().find(|row| row.clip_id == target) {
            selected.insert(target.clone());
            clips.push(Clip {
                id: target,
                saved: row.saved_file_name.clone(),
            });
        }
    }
    Ok(clips)
}

async fn related_media_jobs(
    pool: &Pool<Postgres>,
    guild_id: i64,
    session_id: i64,
) -> Result<Vec<MediaJob>, AppError> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM media_jobs WHERE guild_id=$1 AND request->>'session_id'=$2",
        guild_id,
        session_id.to_string()
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|id| MediaJob { id })
    .collect())
}

async fn related_composition_jobs(
    pool: &Pool<Postgres>,
    job: &Claimed,
    clips: &[Clip],
) -> Result<Vec<CompositionJob>, AppError> {
    let clip_ids: HashSet<&str> = clips.iter().map(|clip| clip.id.as_str()).collect();
    let rows = sqlx::query!(
        "SELECT id,result_clip_id,snapshot FROM composition_jobs WHERE guild_id=$1",
        job.guild_id
    )
    .fetch_all(pool)
    .await?;
    let mut jobs = Vec::new();
    for row in rows {
        let dependent = row
            .snapshot
            .get("body")
            .and_then(|body| body.get("segments"))
            .and_then(|segments| segments.as_array())
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    segment
                        .get("source_id")
                        .and_then(|id| id.as_str())
                        .is_some_and(|id| clip_ids.contains(id))
                })
            });
        if clip_ids.contains(row.result_clip_id.as_str()) || dependent {
            jobs.push(CompositionJob { id: row.id });
        }
    }
    Ok(jobs)
}

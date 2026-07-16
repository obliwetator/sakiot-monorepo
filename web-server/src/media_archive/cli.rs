use std::{error::Error, time::Duration};

use futures_util::{StreamExt, stream};
use sakiot_storage::{FileDigest, StorageErrorKind, hash_file};
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

use super::{MediaArchive, repository, worker};

type CliResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub async fn run_media_command(arguments: &[String]) -> CliResult<()> {
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    match arguments {
        [command] if command == "status" => print_status(&pool).await?,
        [command, flag] if command == "migrate" && flag == "--dry-run" => {
            print_dry_run(&pool).await?
        }
        [command, flag] if command == "migrate" && flag == "--wait" => {
            let media = require_archive().await?;
            repository::reconcile(&pool).await?;
            repository::requeue_missing(&pool).await?;
            wait_for_queue(&pool, &media).await?;
            ensure_clean_terminal_state(&pool).await?;
            print_status(&pool).await?;
        }
        [command] if command == "verify" => {
            let media = require_archive().await?;
            repository::reconcile(&pool).await?;
            repository::requeue_missing(&pool).await?;
            wait_for_queue(&pool, &media).await?;
            let objects = repository::list_available(&pool).await?;
            let verified = verify_available_objects(&pool, &media, objects).await?;
            ensure_clean_terminal_state(&pool).await?;
            output(&format!(
                "full verification complete for {verified} available object(s)"
            ));
            print_status(&pool).await?;
        }
        [command, flag] if command == "restore" && flag == "--all" => {
            let media = require_archive().await?;
            let objects = repository::list_available(&pool).await?;
            restore_objects(&pool, &media, objects).await?;
            print_status(&pool).await?;
        }
        [command, flag, ids] if command == "restore" && flag == "--audio-file-ids" => {
            let media = require_archive().await?;
            let ids = parse_ids(ids)?;
            let objects = repository::list_available_recordings(&pool, &ids).await?;
            restore_objects(&pool, &media, objects).await?;
            let mut unavailable = Vec::new();
            for id in &ids {
                let path = repository::recording_source_path(&pool, *id).await?;
                let exists = match path {
                    Some(path) => tokio::fs::try_exists(path).await.unwrap_or(false),
                    None => false,
                };
                if !exists {
                    unavailable.push(*id);
                }
            }
            if !unavailable.is_empty() {
                return Err(format!(
                    "selected recording(s) are neither local nor available in the archive: {unavailable:?}"
                )
                .into());
            }
            print_status(&pool).await?;
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

async fn require_archive() -> CliResult<MediaArchive> {
    let media = MediaArchive::from_env().await?;
    if !media.enabled() {
        return Err("media archive is disabled".into());
    }
    Ok(media)
}

async fn print_status(pool: &Pool<Postgres>) -> CliResult<()> {
    let eligible = repository::eligible_status(pool).await?;
    let status = repository::status(pool).await?;
    output(&format!(
        "eligible: {} recording(s), {} clip(s); tracked: {}",
        eligible.recordings, eligible.clips, eligible.tracked
    ));
    output(&format!(
        "pending: {} object(s), {} byte(s); uploading: {}; oldest backlog: {}s",
        status.pending_objects,
        status.pending_bytes,
        status.uploading_objects,
        status.oldest_backlog_seconds
    ));
    output(&format!(
        "available: {} object(s), {} byte(s); missing: {}; conflict: {}",
        status.available_objects,
        status.available_bytes,
        status.missing_objects,
        status.conflict_objects
    ));
    Ok(())
}

async fn print_dry_run(pool: &Pool<Postgres>) -> CliResult<()> {
    let eligible = repository::eligible_status(pool).await?;
    output(&format!(
        "dry run: {} finalized recording(s) + {} saved clip(s) = {} eligible object(s); {} already tracked",
        eligible.recordings,
        eligible.clips,
        eligible.recordings + eligible.clips,
        eligible.tracked
    ));
    print_status(pool).await
}

async fn wait_for_queue(pool: &Pool<Postgres>, media: &MediaArchive) -> CliResult<()> {
    let owner = worker::lease_owner();
    loop {
        let processed = worker::process_available_batch(pool, media, &owner).await?;
        let status = repository::status(pool).await?;
        super::metrics::record_status(&status);
        if status.pending_objects == 0 && status.uploading_objects == 0 {
            return Ok(());
        }
        if processed == 0 {
            let delay = repository::next_retry_at(pool)
                .await?
                .map(|retry_at| {
                    (retry_at - chrono::Utc::now())
                        .to_std()
                        .unwrap_or(Duration::from_millis(100))
                        .clamp(Duration::from_millis(100), Duration::from_secs(15))
                })
                .unwrap_or(Duration::from_secs(1));
            tokio::time::sleep(delay).await;
        }
    }
}

async fn ensure_clean_terminal_state(pool: &Pool<Postgres>) -> CliResult<()> {
    let status = repository::status(pool).await?;
    if status.missing_objects > 0 || status.conflict_objects > 0 {
        return Err(format!(
            "archive incomplete: {} missing object(s), {} conflict object(s)",
            status.missing_objects, status.conflict_objects
        )
        .into());
    }
    Ok(())
}

async fn restore_objects(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    objects: Vec<repository::AvailableObject>,
) -> CliResult<()> {
    let archive = media.archive().ok_or("media archive is disabled")?;
    let total = objects.len();
    let mut restored = 0usize;
    for object in objects {
        let expected = FileDigest {
            bytes: object.bytes,
            sha256: object.sha256.clone(),
        };
        let local_matches = match hash_file(&object.path).await {
            Ok(actual) => actual == expected,
            Err(_) => false,
        };
        if !local_matches {
            archive
                .download_verified(&object.object_key, &object.path, &expected)
                .await?;
            restored += 1;
        }
        repository::reset_local_retention(
            pool,
            object.id,
            media
                .config()
                .map_or(7, |config| config.local_retention_days),
        )
        .await?;
    }
    output(&format!(
        "restore complete: {restored} downloaded, {} already matched, {total} verified",
        total - restored
    ));
    Ok(())
}

async fn verify_available_objects(
    pool: &Pool<Postgres>,
    media: &MediaArchive,
    objects: Vec<repository::AvailableObject>,
) -> CliResult<usize> {
    let archive = media.archive().ok_or("media archive is disabled")?.clone();
    let retention_days = media
        .config()
        .map_or(7, |config| config.local_retention_days);
    let total = objects.len();
    let tasks = stream::iter(objects.into_iter().map(|object| {
        let archive = archive.clone();
        let pool = pool.clone();
        async move {
            let expected = FileDigest {
                bytes: object.bytes,
                sha256: object.sha256.clone(),
            };
            let started = std::time::Instant::now();
            match archive.verify_object(&object.object_key, &expected).await {
                Ok(head) => {
                    super::metrics::transfer_success("verify", expected.bytes, started.elapsed());
                    if !repository::refresh_available_verification(
                        &pool,
                        object.id,
                        head.etag.as_deref(),
                        retention_days,
                    )
                    .await?
                    {
                        return Err::<(), Box<dyn Error + Send + Sync>>(
                            format!(
                                "media object {} changed state during verification",
                                object.id
                            )
                            .into(),
                        );
                    }
                    Ok(())
                }
                Err(error) if error.kind() == StorageErrorKind::Integrity => {
                    super::metrics::verification_failure();
                    repository::mark_verification_conflict(&pool, object.id, &error.to_string())
                        .await?;
                    Ok(())
                }
                Err(error) if error.kind() == StorageErrorKind::NotFound => {
                    super::metrics::verification_failure();
                    repository::mark_remote_missing(&pool, object.id, &error.to_string()).await?;
                    Ok(())
                }
                Err(error) => {
                    super::metrics::verification_failure();
                    Err(error.into())
                }
            }
        }
    }))
    .buffer_unordered(2);
    futures_util::pin_mut!(tasks);
    while let Some(result) = tasks.next().await {
        result?;
    }
    Ok(total)
}

fn parse_ids(value: &str) -> CliResult<Vec<i64>> {
    let mut ids = value
        .split(',')
        .map(|part| part.parse::<i64>())
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() || ids.iter().any(|id| *id <= 0) {
        return Err("--audio-file-ids requires positive comma-separated ids".into());
    }
    Ok(ids)
}

fn usage() -> &'static str {
    "usage: web_server media status | media migrate --dry-run | media migrate --wait | media verify | media restore --all"
}

#[expect(clippy::print_stdout, reason = "operator CLI writes requested results")]
fn output(message: &str) {
    println!("{message}");
}

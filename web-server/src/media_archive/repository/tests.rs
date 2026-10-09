use sqlx::PgPool;

use super::*;

async fn seed_sources(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audio_files
                (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
             VALUES
                ('media-finalized', 1, 10, 100, 2026, 7, 1000, 2000),
                ('media-active', 1, 10, 100, 2026, 7, 3000, NULL)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO clips (clip_id, start_time, saved_file_name)
             VALUES
                ('media-saved-clip', 0, '2026/07/media-saved-clip.ogg'),
                ('media-unsaved-clip', 0, NULL)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn archive_snapshot_keeps_the_old_path_after_an_overwrite(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_sources(&pool).await?;
    reconcile(&pool).await?;
    sqlx::query("UPDATE media_objects SET state = 'available', object_key = 'test', bytes = 1, sha256 = repeat('a',64), uploaded_at = now(), verified_at = now(), local_delete_after = now() WHERE clip_id = 'media-saved-clip'").execute(&pool).await?;
    let row = sqlx::query_as!(
        AvailableMediaRow,
        r#"SELECT id,
                      audio_file_id,
                      clip_id,
                      clip_saved_file_name,
                      object_key AS "object_key!",
                      bytes AS "bytes!",
                      sha256 AS "sha256!",
                      local_delete_after AS "local_delete_after!"
                 FROM media_objects
                WHERE clip_id = 'media-saved-clip'"#
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "UPDATE clips SET saved_file_name = 'replacement.ogg' WHERE clip_id = 'media-saved-clip'",
    )
    .execute(&pool)
    .await?;
    let stale_cleanup_snapshot = available_from_row(&pool, row).await?;
    assert!(
        stale_cleanup_snapshot
            .path
            .ends_with("2026/07/media-saved-clip.ogg")
    );
    assert!(!stale_cleanup_snapshot.path.ends_with("replacement.ogg"));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn reconciliation_queues_only_finalized_or_saved_sources(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_sources(&pool).await?;
    assert_eq!(reconcile(&pool).await?, 2);
    assert_eq!(reconcile(&pool).await?, 0);
    let sources: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT af.file_name, mo.clip_id
               FROM media_objects mo
               LEFT JOIN audio_files af ON af.id = mo.audio_file_id
              ORDER BY COALESCE(af.file_name, mo.clip_id)",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        sources,
        vec![
            (Some("media-finalized".to_owned()), None),
            (None, Some("media-saved-clip".to_owned())),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn reconciliation_skips_synthetic_load_test_media(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_sources(&pool).await?;
    let synthetic = crate::synthetic::SYNTHETIC_ID_FLOOR + 1;
    sqlx::query(
        "INSERT INTO audio_files
                (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
             VALUES ('synthetic-finalized', $1, $1, $1, 2026, 7, 1000, 2000)",
    )
    .bind(synthetic)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO clips (clip_id, guild_id, start_time, saved_file_name)
             VALUES ('synthetic-clip', $1, 0, '2026/07/synthetic-clip.ogg')",
    )
    .bind(synthetic)
    .execute(&pool)
    .await?;

    assert_eq!(reconcile(&pool).await?, 2);
    let synthetic_tracked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM media_objects mo
               LEFT JOIN audio_files af ON af.id = mo.audio_file_id
              WHERE af.file_name = 'synthetic-finalized' OR mo.clip_id = 'synthetic-clip'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(synthetic_tracked, 0);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn claims_are_exclusive_and_expired_leases_recover(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_sources(&pool).await?;
    reconcile(&pool).await?;
    let first = claim_batch(&pool, "worker-a", 1).await?;
    let second = claim_batch(&pool, "worker-b", 1).await?;
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].id, second[0].id);
    assert!(claim_batch(&pool, "worker-c", 1).await?.is_empty());

    sqlx::query(
        "UPDATE media_objects
                SET lease_expires_at = now() - interval '1 second'
              WHERE id = $1",
    )
    .bind(first[0].id)
    .execute(&pool)
    .await?;
    let recovered = claim_batch(&pool, "worker-c", 1).await?;
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, first[0].id);
    assert_eq!(recovered[0].attempts, 2);
    mark_pending(&pool, &recovered[0], "worker-c", "temporary B2 outage").await?;
    let retry: (
        String,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT state, lease_owner, retry_at, last_error
                   FROM media_objects
                  WHERE id = $1",
    )
    .bind(recovered[0].id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(retry.0, "pending");
    assert_eq!(retry.1, None);
    assert!(retry.2.is_some_and(|retry_at| retry_at > Utc::now()));
    assert_eq!(retry.3.as_deref(), Some("temporary B2 outage"));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn full_reverification_keeps_media_available(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_sources(&pool).await?;
    reconcile(&pool).await?;
    let id: i64 = sqlx::query_scalar(
        "UPDATE media_objects
                SET state = 'available',
                    object_key = 'media/v1/recordings/1/test.ogg',
                    bytes = 4,
                    sha256 = repeat('a', 64),
                    uploaded_at = now() - interval '1 day',
                    verified_at = now() - interval '1 day',
                    local_delete_after = now()
              WHERE audio_file_id IS NOT NULL
              RETURNING id",
    )
    .fetch_one(&pool)
    .await?;

    assert!(refresh_available_verification(&pool, id, Some("\"new-etag\""), 7).await?);
    let (state, etag, delete_after): (String, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT state, etag, local_delete_after
                   FROM media_objects
                  WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, "available");
    assert_eq!(etag.as_deref(), Some("\"new-etag\""));
    assert!(delete_after > Utc::now() + chrono::Duration::days(6));
    Ok(())
}

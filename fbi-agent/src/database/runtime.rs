use crate::cast::ToI64;
use sakiot_paths::{DataRoots, RecordingKey};
use serenity::model::id::{ChannelId, GuildId};
use sqlx::{Pool, Postgres};
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::database::DbResult;
use crate::runtime::RuntimeState;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VoiceLeaseClaim {
    Claimed,
    OwnedByOther(String),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StoppedInstanceCleanup {
    pub leases_deleted: u64,
    pub recordings_closed: u64,
    pub instances_updated: u64,
}

/// End timestamp for a recording whose owner stopped. The source file's last
/// write time is the closest honest estimate of when audio stopped; when the
/// file is gone, the start timestamp stays as the documented sentinel for
/// "closed without a measured end" (a zero-length recording).
fn recording_end_ts(start_ts: Option<i64>, file_modified_ms: Option<i64>) -> Option<i64> {
    let start = start_ts?;
    Some(match file_modified_ms {
        Some(modified) => start.max(modified),
        None => start,
    })
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    i64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

pub async fn upsert_instance(pool: &Pool<Postgres>, runtime: &RuntimeState) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO bot_instances (instance_id, role, state, grpc_address, heartbeat_at, started_at)
         VALUES ($1, $2, $3, $4, now(), now())
         ON CONFLICT (instance_id) DO UPDATE
            SET role = EXCLUDED.role,
                state = EXCLUDED.state,
                grpc_address = EXCLUDED.grpc_address,
                heartbeat_at = now()",
        runtime.config().instance_id,
        runtime.role().as_str(),
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        },
        runtime.config().grpc_address.as_deref()
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn heartbeat_instance_and_leases(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
) -> DbResult<()> {
    upsert_instance(pool, runtime).await?;

    sqlx::query!(
        "UPDATE voice_session_leases
            SET state = $2, heartbeat_at = now()
          WHERE owner_instance_id = $1",
        runtime.config().instance_id,
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        }
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn claim_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> DbResult<VoiceLeaseClaim> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let result = sqlx::query!(
        "INSERT INTO voice_session_leases
            (guild_id, channel_id, owner_instance_id, state, heartbeat_at, started_at)
         VALUES ($1, $2, $3, $4, now(), now())
         ON CONFLICT (guild_id) DO UPDATE
            SET channel_id = EXCLUDED.channel_id,
                owner_instance_id = EXCLUDED.owner_instance_id,
                state = EXCLUDED.state,
                heartbeat_at = now()
          WHERE voice_session_leases.owner_instance_id = EXCLUDED.owner_instance_id
             OR voice_session_leases.heartbeat_at <= now() - ($5::double precision * interval '1 second')
             OR NOT EXISTS (
                SELECT 1
                  FROM bot_instances b
                 WHERE b.instance_id = voice_session_leases.owner_instance_id
                   AND b.heartbeat_at > now() - ($5::double precision * interval '1 second')
                   AND b.state <> 'stopped'
             )",
        guild_id.to_i64(),
        channel_id.to_i64(),
        runtime.config().instance_id,
        if runtime.is_draining() {
            "draining"
        } else {
            "active"
        },
        stale_after_seconds
    )
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        return Ok(VoiceLeaseClaim::Claimed);
    }

    Ok(VoiceLeaseClaim::OwnedByOther(
        active_lease_owner(pool, guild_id)
            .await?
            .unwrap_or_else(|| "unknown".to_string()),
    ))
}

pub async fn release_voice_session(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
) -> DbResult<u64> {
    let result = sqlx::query!(
        "DELETE FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
        guild_id.to_i64(),
        runtime.config().instance_id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn update_voice_session_channel(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> DbResult<u64> {
    let result = sqlx::query(
        "UPDATE voice_session_leases
            SET channel_id = $3, heartbeat_at = now(), updated_at = now()
          WHERE guild_id = $1 AND owner_instance_id = $2",
    )
    .bind(guild_id.to_i64())
    .bind(&runtime.config().instance_id)
    .bind(channel_id.to_i64())
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn active_lease_owner(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
) -> DbResult<Option<String>> {
    let stale_after_seconds = crate::heartbeat::STALE_AFTER_SECONDS as f64;
    let row = sqlx::query_scalar!(
        "SELECT v.owner_instance_id
           FROM voice_session_leases v
           JOIN bot_instances b ON b.instance_id = v.owner_instance_id
          WHERE v.guild_id = $1
            AND v.heartbeat_at > now() - ($2::double precision * interval '1 second')
            AND b.heartbeat_at > now() - ($2::double precision * interval '1 second')
            AND b.state <> 'stopped'
          LIMIT 1",
        guild_id.to_i64(),
        stale_after_seconds
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

pub async fn mark_instance_stopped(
    pool: &Pool<Postgres>,
    runtime: &RuntimeState,
) -> DbResult<StoppedInstanceCleanup> {
    let leases_deleted = sqlx::query!(
        "DELETE FROM voice_session_leases
          WHERE owner_instance_id = $1",
        runtime.config().instance_id
    )
    .execute(pool)
    .await?
    .rows_affected();

    // Close every recording this instance still owns. A stopped instance
    // never measured an end, so `COALESCE(end_ts, start_ts)` used to record a
    // zero-length recording; the source file's last write time is the closest
    // honest estimate of when audio stopped.
    let recordings_root = DataRoots::from_env().recordings_str();
    let open_recordings = sqlx::query!(
        "SELECT id, guild_id, channel_id, year, month, file_name, start_ts
           FROM audio_files
          WHERE recording_owner_instance_id = $1
            AND end_ts IS NULL",
        runtime.config().instance_id
    )
    .fetch_all(pool)
    .await?;
    let mut recordings_closed = 0;
    for recording in open_recordings {
        let key = RecordingKey::new(
            recording.guild_id,
            recording.channel_id,
            recording.year,
            recording.month as u32,
            recording.file_name.clone(),
        );
        let end_ts = recording_end_ts(
            recording.start_ts,
            file_modified_ms(&key.recording_path(&recordings_root)),
        );
        recordings_closed += sqlx::query!(
            "UPDATE audio_files
                SET end_ts = $2,
                    reaped = TRUE,
                    recording_heartbeat_at = NULL,
                    finalize_reason_id = COALESCE(finalize_reason_id, 3)
              WHERE id = $1
                AND end_ts IS NULL
                AND recording_owner_instance_id = $3",
            recording.id,
            end_ts,
            runtime.config().instance_id
        )
        .execute(pool)
        .await?
        .rows_affected();
    }

    sqlx::query(
        "UPDATE recording_sessions rs
            SET state = 'finalized',
                ended_at = COALESCE(
                    (
                        SELECT to_timestamp(COALESCE(MAX(af.end_ts), MAX(af.start_ts))::double precision / 1000.0)
                          FROM audio_files af
                         WHERE af.recording_session_id = rs.id
                    ),
                    rs.started_at
                ),
                owner_instance_id = NULL,
                end_reason = COALESCE(end_reason, 'instance_stopped'),
                updated_at = now()
          WHERE rs.owner_instance_id = $1 AND rs.state = 'active'",
    )
    .bind(&runtime.config().instance_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "UPDATE recording_sessions
            SET owner_instance_id = NULL, updated_at = now()
          WHERE owner_instance_id = $1 AND state = 'pending'",
    )
    .bind(&runtime.config().instance_id)
    .execute(pool)
    .await?;

    let instances_updated = sqlx::query!(
        "UPDATE bot_instances
            SET state = 'stopped', heartbeat_at = now()
          WHERE instance_id = $1",
        runtime.config().instance_id
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(StoppedInstanceCleanup {
        leases_deleted,
        recordings_closed,
        instances_updated,
    })
}

#[cfg(test)]
mod tests {
    use super::{file_modified_ms, recording_end_ts};

    #[test]
    fn stopped_recordings_use_the_file_mtime_over_the_start() {
        // The file outlived the start: its last write is the better end.
        assert_eq!(recording_end_ts(Some(1_000), Some(9_000)), Some(9_000));
        // A clock skew that puts the mtime before the start must not produce a
        // negative duration.
        assert_eq!(recording_end_ts(Some(9_000), Some(1_000)), Some(9_000));
    }

    #[test]
    fn missing_files_keep_the_start_as_the_documented_sentinel() {
        assert_eq!(recording_end_ts(Some(1_000), None), Some(1_000));
        assert_eq!(recording_end_ts(None, Some(9_000)), None);
        assert_eq!(recording_end_ts(None, None), None);
    }

    #[test]
    fn file_modified_ms_reads_real_files_only() {
        let dir =
            std::env::temp_dir().join(format!("sakiot-runtime-test-{}", std::process::id() as u64));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recording.ogg");
        std::fs::write(&path, b"audio").unwrap();
        assert!(file_modified_ms(&path).is_some());
        assert!(file_modified_ms(&dir.join("missing.ogg")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

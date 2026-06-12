use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serenity::{
    model::id::{ChannelId, GuildId},
    prelude::{RwLock, TypeMap},
};
use sqlx::PgPool;

use crate::cooldown::JamCooldown;
use crate::database::{DbError, recordings};
use crate::runtime::{BotRole, RuntimeConfig, RuntimeState};

fn unique_id() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after unix epoch")
        .as_millis() as i64;
    9_000_000_000_000 + (millis % 1_000_000_000)
}

async fn insert_test_instance(
    pool: &PgPool,
    owner: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query!(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
         VALUES ($1, 'active', 'active', now(), now())
         ON CONFLICT (instance_id) DO UPDATE SET state = 'active', heartbeat_at = now()",
        owner
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn test_runtime(instance_id: String) -> Arc<RuntimeState> {
    RuntimeState::new(RuntimeConfig {
        instance_id,
        initial_role: BotRole::Active,
        drain_timeout: Duration::from_secs(30),
    })
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_create_heartbeat_finalize_uses_audio_file_id(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let base = unique_id();
    let guild_id = base;
    let channel_id = base + 1;
    let user_id = base + 2;
    let owner = format!("test-recording-{base}");
    insert_test_instance(&pool, &owner).await?;

    let handle = recordings::create_recording_for_test(
        &pool,
        guild_id,
        channel_id,
        user_id,
        chrono::Utc::now(),
        &owner,
        temporary.path(),
    )
    .await?;

    assert!(handle.audio_file_id > 0);
    assert!(!handle.file_name.is_empty());
    assert_eq!(
        recordings::heartbeat_active_recordings(&pool, &[handle.audio_file_id], &owner).await?,
        1
    );
    recordings::finalize_recording(
        &pool,
        handle.audio_file_id,
        &owner,
        1_234,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await?;

    let row = sqlx::query!(
        "SELECT id, file_name, end_ts - start_ts AS duration_ms, finalize_reason_id
           FROM audio_files
          WHERE id = $1",
        handle.audio_file_id
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(row.id, handle.audio_file_id);
    assert_eq!(row.file_name, handle.file_name);
    assert_eq!(row.duration_ms, Some(1_234));
    assert_eq!(
        row.finalize_reason_id,
        Some(recordings::FINALIZE_REASON_WRITER_CLOSE)
    );

    sqlx::query!(
        "DELETE FROM audio_files WHERE id = $1",
        handle.audio_file_id
    )
    .execute(&pool)
    .await?;
    sqlx::query!("DELETE FROM bot_instances WHERE instance_id = $1", owner)
        .execute(&pool)
        .await?;

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_finalize_reports_zero_row_mismatch(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let err = recordings::finalize_recording(
        &pool,
        -unique_id(),
        "missing-owner",
        1,
        recordings::FINALIZE_REASON_WRITER_CLOSE,
    )
    .await
    .expect_err("missing recording should report row mismatch");

    assert!(matches!(
        err,
        DbError::UnexpectedRows {
            operation: "finalize recording",
            expected: 1,
            actual: 0
        }
    ));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn cooldown_db_failure_propagates(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    pool.close().await;

    let cooldown = JamCooldown::new();
    let result = cooldown.check_and_record(&pool, 1, 2).await;
    assert!(result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn db_constraints_reject_negative_cooldown(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();

    let negative_cooldown = sqlx::query!(
        "INSERT INTO guild_jam_cooldowns (guild_id, cooldown_seconds)
         VALUES ($1, -1)",
        guild_id
    )
    .execute(&pool)
    .await;
    assert!(negative_cooldown.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn guild_cache_accepts_unknown_discord_channel_types(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();
    let channel_id = guild_id + 1;

    sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, $2)")
        .bind(guild_id)
        .bind(guild_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 255, 'future-channel-type')",
    )
    .bind(channel_id)
    .bind(guild_id)
    .execute(&pool)
    .await?;

    let channel_type =
        sqlx::query_scalar::<_, i32>("SELECT type FROM channels WHERE channel_id = $1")
            .bind(channel_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(channel_type, 255);

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn guild_cache_prune_removes_stale_roles_channels_and_dependents(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = unique_id();
    let owner_id = guild_id + 1;
    let keep_role = guild_id + 10;
    let stale_role = guild_id + 11;
    let keep_channel = guild_id + 20;
    let stale_channel = guild_id + 21;
    let user_id = guild_id + 30;

    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2)",
        guild_id,
        owner_id
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channel_type (id, type)
         VALUES (2, 'voice')
         ON CONFLICT (id) DO NOTHING"
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $2, 0, 'keep'), ($1, $3, 0, 'stale')",
        guild_id,
        keep_role,
        stale_role
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id)
         VALUES ($1, $2), ($1, $3)",
        user_id,
        keep_role,
        stale_role
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'keep'), ($3, $2, 2, 'stale')",
        keep_channel,
        guild_id,
        stale_channel
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $3, 'role', 0, 0), ($2, $3, 'role', 0, 0)",
        keep_channel,
        stale_channel,
        keep_role
    )
    .execute(&pool)
    .await?;

    crate::database::guild_cache::prune_stale_roles_for_test(&pool, guild_id, &[keep_role]).await?;
    crate::database::guild_cache::prune_stale_channels_for_test(&pool, guild_id, &[keep_channel])
        .await?;

    let stale_roles =
        sqlx::query_scalar!("SELECT COUNT(*) FROM roles WHERE role_id = $1", stale_role)
            .fetch_one(&pool)
            .await?
            .unwrap_or(0);
    let keep_roles =
        sqlx::query_scalar!("SELECT COUNT(*) FROM roles WHERE role_id = $1", keep_role)
            .fetch_one(&pool)
            .await?
            .unwrap_or(0);
    let stale_user_roles = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_roles WHERE role_id = $1",
        stale_role
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_user_roles = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_roles WHERE role_id = $1",
        keep_role
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let stale_channels = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channels WHERE channel_id = $1",
        stale_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_channels = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channels WHERE channel_id = $1",
        keep_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let stale_permissions = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channel_permissions WHERE channel_id = $1",
        stale_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);
    let keep_permissions = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM channel_permissions WHERE channel_id = $1",
        keep_channel
    )
    .fetch_one(&pool)
    .await?
    .unwrap_or(0);

    assert_eq!(stale_roles, 0);
    assert_eq!(keep_roles, 1);
    assert_eq!(stale_user_roles, 0);
    assert_eq!(keep_user_roles, 1);
    assert_eq!(stale_channels, 0);
    assert_eq!(keep_channels, 1);
    assert_eq!(stale_permissions, 0);
    assert_eq!(keep_permissions, 1);

    sqlx::query!("DELETE FROM guilds WHERE id = $1", guild_id)
        .execute(&pool)
        .await?;

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn local_disconnect_releases_only_current_owner_lease(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = unique_id();
    let own_guild = GuildId::new(base as u64);
    let other_guild = GuildId::new((base + 1) as u64);
    let own_runtime = test_runtime(format!("test-local-disconnect-{base}"));
    let other_runtime = test_runtime(format!("test-other-owner-{base}"));

    crate::deployment::upsert_instance(&pool, &own_runtime).await?;
    crate::deployment::upsert_instance(&pool, &other_runtime).await?;
    crate::deployment::claim_voice_session(
        &pool,
        &own_runtime,
        own_guild,
        ChannelId::new((base + 10) as u64),
    )
    .await?;
    crate::deployment::claim_voice_session(
        &pool,
        &other_runtime,
        other_guild,
        ChannelId::new((base + 11) as u64),
    )
    .await?;

    let data = Arc::new(RwLock::new(TypeMap::new()));
    {
        let mut data_write = data.write().await;
        data_write.insert::<songbird::SongbirdKey>(songbird::Songbird::serenity());
        data_write.insert::<crate::runtime::RuntimeStateKey>(own_runtime.clone());
    }

    let report = crate::events::voice::teardown_voice_session(&data, &pool, own_guild).await;
    assert!(!report.had_call);
    assert!(!report.connected_after);

    let own_leases = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
    )
    .bind(own_guild.get() as i64)
    .bind(&own_runtime.config().instance_id)
    .fetch_one(&pool)
    .await?;
    let other_leases = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM voice_session_leases
          WHERE guild_id = $1 AND owner_instance_id = $2",
    )
    .bind(other_guild.get() as i64)
    .bind(&other_runtime.config().instance_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(own_leases, 0);
    assert_eq!(other_leases, 1);
    Ok(())
}

use crate::cooldown::{CheckResult, JamCooldown};
use sakiot_paths::DataRoots;
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn test_audio_paths_are_valid() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let roots = DataRoots::new(temporary.path());
    let rec_path = roots.recordings;
    let clips_path = roots.clips;

    // Ensure directories can be created
    std::fs::create_dir_all(&rec_path)?;
    std::fs::create_dir_all(&clips_path)?;

    assert!(
        rec_path.exists(),
        "Recording path must exist after creation"
    );
    assert!(clips_path.exists(), "Clips path must exist after creation");

    // Test writing and reading to ensure we have valid paths
    let test_rec_file = rec_path.join("test_write.txt");
    std::fs::write(&test_rec_file, "test_data")?;
    let read_back = std::fs::read_to_string(&test_rec_file)?;
    assert_eq!(read_back, "test_data");

    let test_clip_file = clips_path.join("test_clip.txt");
    std::fs::write(&test_clip_file, "test_clip_data")?;
    let read_clip_back = std::fs::read_to_string(&test_clip_file)?;
    assert_eq!(read_clip_back, "test_clip_data");

    // Clean up
    std::fs::remove_file(test_rec_file)?;
    std::fs::remove_file(test_clip_file)?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn test_jam_cooldown_system(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    // Generate unique pseudo-random IDs to prevent test collision
    let now_millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_millis();
    let test_guild_id = (now_millis % 1_000_000_000) as i64 + 10_000_000_000;
    let test_user_id = test_guild_id + 1;

    let cooldown_manager = JamCooldown::new();

    // 1. Initially, with no database entries, cooldown should be 0 (no cooldown active)
    let res = cooldown_manager
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::Allowed));

    // Subsequent immediate checks should still be Allowed because cooldown is 0
    let res2 = cooldown_manager
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res2, CheckResult::Allowed));

    // 2. Insert guild base cooldown of 3 seconds
    sqlx::query!(
        "INSERT INTO guild_jam_cooldowns (guild_id, cooldown_seconds) VALUES ($1, 3)",
        test_guild_id
    )
    .execute(&pool)
    .await?;

    // Clear memory cache so a fresh lookup is performed
    let cooldown_manager_guild = JamCooldown::new();

    // First check should be allowed
    let res = cooldown_manager_guild
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::Allowed));

    // Immediate second check should be OnCooldown
    let res = cooldown_manager_guild
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::OnCooldown { .. }));

    // Wait 3.5 seconds
    tokio::time::sleep(Duration::from_millis(3500)).await;

    // Third check should be allowed
    let res = cooldown_manager_guild
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::Allowed));

    // 3. Insert user override cooldown of 1 second (which overrides the guild cooldown)
    sqlx::query!(
        "INSERT INTO user_jam_cooldown_overrides (guild_id, user_id, cooldown_seconds) VALUES ($1, $2, 1)",
        test_guild_id,
        test_user_id
    )
    .execute(&pool)
    .await?;

    // Clear memory cache again
    let cooldown_manager_override = JamCooldown::new();

    // First check should be allowed
    let res = cooldown_manager_override
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::Allowed));

    // Immediate second check should be OnCooldown
    let res = cooldown_manager_override
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::OnCooldown { .. }));

    // Wait 1.5 seconds (more than user override of 1s, but less than guild cooldown of 3s)
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Check should be allowed, showing that the 1s override was preferred over the 3s guild cooldown
    let res = cooldown_manager_override
        .check_and_record(&pool, test_guild_id, test_user_id)
        .await?;
    assert!(matches!(res, CheckResult::Allowed));

    // Teardown: Clean up the test records
    sqlx::query!(
        "DELETE FROM user_jam_cooldown_overrides WHERE guild_id = $1 AND user_id = $2",
        test_guild_id,
        test_user_id
    )
    .execute(&pool)
    .await?;

    sqlx::query!(
        "DELETE FROM guild_jam_cooldowns WHERE guild_id = $1",
        test_guild_id
    )
    .execute(&pool)
    .await?;

    Ok(())
}

use crate::cooldown::{CheckResult, JamCooldown};
use crate::permissions::Listening;
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
async fn clip_lookups_take_every_channel_of_anyone_elses_audio(
    pool: sqlx::PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let guild_id = 42_i64;
    let allowed_channel_id = 100_i64;
    let denied_channel_id = 200_i64;
    let user_id = 300_i64;

    let session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES ($1, $2, $3, $4, 'finalized', now(), now(), 'test', 1)
         RETURNING id",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(allowed_channel_id)
    .bind(denied_channel_id)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             recording_session_id, segment_index)
         VALUES
            ('clip-source-allowed', $1, $2, $4, 2026, 8, $5, 0),
            ('clip-source-denied', $1, $3, $4, 2026, 8, $5, 1)",
    )
    .bind(guild_id)
    .bind(allowed_channel_id)
    .bind(denied_channel_id)
    .bind(user_id)
    .bind(session_id)
    .execute(&pool)
    .await?;
    // Every clip of the session names its starting channel; what its audio
    // came from is `channel_ids`.
    sqlx::query(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, channel_ids, speaker_ids, recording_session_id,
             user_id, saved_file_name, start_time, name)
         VALUES
            ('allowed-clip', $1, $2, ARRAY[$2::bigint], ARRAY[$4::bigint], NULL, $4,
             'allowed.ogg', 0, 'Allowed'),
            ('denied-clip', $1, $3, ARRAY[$3::bigint], ARRAY[$4::bigint], NULL, $4,
             'denied.ogg', 0, 'Denied'),
            ('mixed-clip', $1, $2, ARRAY[$2::bigint, $3::bigint], ARRAY[$4::bigint], $5, $4,
             'mixed.ogg', 0, 'Mixed'),
            ('allowed-part-clip', $1, $2, ARRAY[$2::bigint], ARRAY[$4::bigint], $5, $4,
             'part.ogg', 0, 'Allowed part'),
            ('denied-part-clip', $1, $2, ARRAY[$3::bigint], ARRAY[$4::bigint], $5, $4,
             'other.ogg', 1, 'Denied part')",
    )
    .bind(guild_id)
    .bind(allowed_channel_id)
    .bind(denied_channel_id)
    .bind(user_id)
    .bind(session_id)
    .execute(&pool)
    .await?;

    // Someone else, who may listen in the allowed channel only.
    let listener = Listening {
        own: Some(user_id + 1),
        channels: vec![allowed_channel_id],
    };
    let visible =
        crate::database::clips::autocomplete_clip_choices(&pool, guild_id, "", &listener).await?;
    assert_eq!(
        visible
            .into_iter()
            .map(|choice| choice.clip_id)
            .collect::<Vec<_>>(),
        vec!["allowed-clip", "allowed-part-clip"]
    );
    let playable = |clip_id: &'static str, listening: Listening| {
        let pool = pool.clone();
        async move {
            crate::database::clips::playable_clip(&pool, guild_id, clip_id, &listening)
                .await
                .map(|clip| clip.is_some())
        }
    };
    assert!(playable("allowed-part-clip", listener.clone()).await?);
    assert!(!playable("denied-part-clip", listener.clone()).await?);
    assert!(!playable("denied-clip", listener.clone()).await?);
    assert!(!playable("mixed-clip", listener.clone()).await?);
    let everywhere = Listening {
        channels: vec![allowed_channel_id, denied_channel_id],
        ..listener
    };
    assert!(playable("mixed-clip", everywhere.clone()).await?);

    // Every clip holds the speaker's own voice alone: theirs to play from
    // any channel, while a member.
    let speaker = Listening {
        own: Some(user_id),
        channels: Vec::new(),
    };
    assert!(playable("denied-clip", speaker.clone()).await?);
    assert!(playable("mixed-clip", speaker.clone()).await?);
    assert_eq!(
        crate::database::clips::autocomplete_clip_choices(&pool, guild_id, "part", &speaker)
            .await?
            .len(),
        2
    );
    assert!(!playable("denied-clip", Listening::default()).await?);

    // Once the speaker keeps their recordings private, a clip holding their
    // voice is theirs alone, wherever anyone else may listen. Clips of
    // unknown voices cannot ask, and follow their channels.
    sqlx::query(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, channel_ids, speaker_ids, recording_session_id,
             user_id, saved_file_name, start_time, name)
         VALUES
            ('duet-clip', $1, $2, ARRAY[$2::bigint], ARRAY[$3::bigint, $3 + 1], NULL, $3,
             'duet.ogg', 0, 'Duet'),
            ('unknown-clip', $1, $2, ARRAY[$2::bigint], '{}', NULL, $3,
             'unknown.ogg', 0, 'Unknown')",
    )
    .bind(guild_id)
    .bind(allowed_channel_id)
    .bind(user_id)
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO recording_private (guild_id, user_id) VALUES ($1, $2)")
        .bind(guild_id)
        .bind(user_id)
        .execute(&pool)
        .await?;
    assert!(!playable("allowed-clip", everywhere.clone()).await?);
    assert!(!playable("duet-clip", everywhere.clone()).await?);
    assert!(playable("unknown-clip", listener.clone()).await?);
    assert_eq!(
        crate::database::clips::autocomplete_clip_choices(&pool, guild_id, "", &everywhere)
            .await?
            .into_iter()
            .map(|choice| choice.clip_id)
            .collect::<Vec<_>>(),
        vec!["unknown-clip"]
    );
    assert!(playable("denied-clip", speaker.clone()).await?);
    let speaker_listening = Listening {
        channels: vec![allowed_channel_id],
        ..speaker
    };
    assert!(playable("duet-clip", speaker_listening).await?);

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

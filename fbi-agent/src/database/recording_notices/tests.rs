use super::{Delivery, DueNotice, claim_due, mark_sent, queue, record_failure};
use crate::database::DbError;
use crate::database::logical_recordings::create_fragment_in;
use crate::database::recordings::RecordingOwner;
use sqlx::PgPool;

const TEST_OWNER: RecordingOwner<'static> = RecordingOwner {
    instance_id: "test",
    recorder_id: "test-recorder",
};

async fn register_owner(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
             VALUES ('test', 'active', 'active', now(), now())",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn notices(pool: &PgPool) -> Result<Vec<(i64, i64, String)>, sqlx::Error> {
    sqlx::query_as("SELECT guild_id, user_id, status FROM recording_notices ORDER BY guild_id")
        .fetch_all(pool)
        .await
}

/// The notice is per guild: being told in one says nothing about another.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn each_guild_gets_its_own_notice_once(pool: PgPool) -> Result<(), DbError> {
    register_owner(&pool).await?;
    let root = tempfile::tempdir().unwrap();
    let record = |guild_id| {
        create_fragment_in(
            &pool,
            guild_id,
            2,
            3,
            chrono::Utc::now(),
            TEST_OWNER,
            root.path(),
        )
    };

    record(1).await?;
    record(1).await?;
    assert_eq!(notices(&pool).await?, vec![(1, 3, "pending".to_string())]);

    let told = claim_due(&pool).await?.expect("the first guild's notice");
    mark_sent(&pool, &told, Delivery::Dm, 77).await?;
    record(1).await?;
    record(9).await?;
    assert_eq!(
        notices(&pool).await?,
        vec![(1, 3, "sent".to_string()), (9, 3, "pending".to_string()),]
    );
    Ok(())
}

/// Opting out in one guild leaves the member's notice in another untouched.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn opting_out_of_one_guild_does_not_silence_another(pool: PgPool) -> Result<(), DbError> {
    register_owner(&pool).await?;
    let root = tempfile::tempdir().unwrap();
    crate::database::opt_outs::set_opted_out(&pool, 1, 3, true).await?;

    let opted_out =
        create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), TEST_OWNER, root.path()).await;
    assert!(matches!(opted_out, Err(DbError::RecordingOptedOut)));
    create_fragment_in(&pool, 9, 2, 3, chrono::Utc::now(), TEST_OWNER, root.path()).await?;

    assert_eq!(notices(&pool).await?, vec![(9, 3, "pending".to_string())]);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_excluded_channel_queues_no_notice(pool: PgPool) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO guild_recording_policy (guild_id,excluded_channel_ids)
         VALUES (1,ARRAY[2::bigint])",
    )
    .execute(&pool)
    .await?;
    let root = tempfile::tempdir().unwrap();
    let result =
        create_fragment_in(&pool, 1, 2, 3, chrono::Utc::now(), TEST_OWNER, root.path()).await;
    assert!(matches!(result, Err(DbError::RecordingExcluded)));
    assert!(notices(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_claimed_notice_is_not_claimed_again_until_it_comes_due(
    pool: PgPool,
) -> Result<(), DbError> {
    queue(&pool, 1, 2, 3).await?;
    let claimed = claim_due(&pool).await?;
    assert_eq!(
        claimed,
        Some(DueNotice {
            guild_id: 1,
            user_id: 3,
            channel_id: 2,
            attempts: 1,
            delivery: Delivery::Dm,
        })
    );
    assert_eq!(claim_due(&pool).await?, None);

    sqlx::query("UPDATE recording_notices SET next_attempt_at = now()")
        .execute(&pool)
        .await?;
    assert_eq!(
        claim_due(&pool).await?.map(|notice| notice.attempts),
        Some(2)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_claim_carries_the_guilds_preference(pool: PgPool) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO guild_recording_policy (guild_id,notice_delivery) VALUES (1,'channel')",
    )
    .execute(&pool)
    .await?;
    queue(&pool, 1, 2, 3).await?;
    assert_eq!(
        claim_due(&pool).await?.map(|notice| notice.delivery),
        Some(Delivery::Channel)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_transient_failure_stays_pending_and_giving_up_does_not(
    pool: PgPool,
) -> Result<(), DbError> {
    queue(&pool, 1, 2, 3).await?;
    let notice = claim_due(&pool).await?.expect("queued");

    record_failure(&pool, &notice, "503", false).await?;
    assert_eq!(notices(&pool).await?, vec![(1, 3, "pending".to_string())]);

    record_failure(&pool, &notice, "403", true).await?;
    assert_eq!(notices(&pool).await?, vec![(1, 3, "failed".to_string())]);
    Ok(())
}

/// Someone the bot could not reach is tried again while they keep being
/// recorded, but not on every fragment.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_failed_notice_is_requeued_only_after_a_day(pool: PgPool) -> Result<(), DbError> {
    queue(&pool, 1, 2, 3).await?;
    let notice = claim_due(&pool).await?.expect("queued");
    record_failure(&pool, &notice, "403", true).await?;

    queue(&pool, 1, 2, 3).await?;
    assert_eq!(notices(&pool).await?, vec![(1, 3, "failed".to_string())]);

    sqlx::query("UPDATE recording_notices SET next_attempt_at = now() - interval '25 hours'")
        .execute(&pool)
        .await?;
    queue(&pool, 1, 5, 3).await?;
    assert_eq!(notices(&pool).await?, vec![(1, 3, "pending".to_string())]);
    let requeued = claim_due(&pool).await?.expect("requeued");
    assert_eq!((requeued.channel_id, requeued.attempts), (5, 1));
    Ok(())
}

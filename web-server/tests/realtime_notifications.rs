//! The `sakiot_realtime` triggers: displayed changes notify, with ids only;
//! no-op writes, heartbeats and rolled-back transactions stay silent.

use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgListener;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const GUILD: i64 = 1;
const CHANNEL: i64 = 100;
const USER: i64 = 10;
const ROLE: i64 = 1001;

struct Notifications {
    listener: PgListener,
    pool: PgPool,
    marker: u32,
}

impl Notifications {
    async fn listen(pool: &PgPool) -> Result<Self, sqlx::Error> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen("sakiot_realtime").await?;
        Ok(Self {
            listener,
            pool: pool.clone(),
            marker: 0,
        })
    }

    /// Everything committed since the last call, in commit order. A marker
    /// notification sent last proves nothing else is still on its way.
    async fn drain(&mut self) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        self.marker += 1;
        let marker = json!({ "marker": self.marker });
        sqlx::query("SELECT pg_notify('sakiot_realtime', $1)")
            .bind(marker.to_string())
            .execute(&self.pool)
            .await?;
        let mut received = Vec::new();
        loop {
            let notification =
                tokio::time::timeout(std::time::Duration::from_secs(5), self.listener.recv())
                    .await??;
            let payload: Value = serde_json::from_str(notification.payload())?;
            if payload == marker {
                return Ok(received);
            }
            received.push(payload);
        }
    }
}

async fn seed(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state) VALUES ('bot', 'active', 'active')",
    )
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, 20)")
        .bind(GUILD)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name) VALUES ($1, $2, 2, 'voice')",
    )
    .bind(CHANNEL)
    .bind(GUILD)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $1, 0, '@everyone'), ($1, $2, 0, 'Role')",
    )
    .bind(GUILD)
    .bind(ROLE)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_session(pool: &PgPool) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state, started_at)
         VALUES ($1, $2, $3, $3, 'active', now())
         RETURNING id",
    )
    .bind(GUILD)
    .bind(USER)
    .bind(CHANNEL)
    .fetch_one(pool)
    .await
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recordings_notify_displayed_changes_only(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;

    let session = insert_session(&pool).await?;
    let fragment: i64 = sqlx::query_scalar(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts,
             recording_session_id, segment_index)
         VALUES ('fragment', $1, $2, $3, 2026, 10, 1000, $4, 0)
         RETURNING id",
    )
    .bind(GUILD)
    .bind(CHANNEL)
    .bind(USER)
    .bind(session)
    .fetch_one(&pool)
    .await?;
    let session_event = json!({
        "v": 1, "k": "session",
        "g": GUILD.to_string(), "s": session.to_string(), "c": CHANNEL.to_string(),
    });
    assert_eq!(
        notifications.drain().await?,
        vec![session_event.clone(), session_event.clone()],
        "a new session and its first fragment"
    );

    // The recording heartbeat (every 10 s) and session bookkeeping are silent.
    sqlx::query("UPDATE audio_files SET recording_heartbeat_at = now() WHERE id = $1")
        .bind(fragment)
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE recording_sessions
            SET updated_at = now(), owner_instance_id = 'bot', pending_deadline_at = now()
          WHERE id = $1",
    )
    .bind(session)
    .execute(&pool)
    .await?;
    // A no-op write of a displayed column is silent too.
    sqlx::query("UPDATE recording_sessions SET state = state WHERE id = $1")
        .bind(session)
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());

    // A rolled-back change is never delivered.
    let mut transaction = pool.begin().await?;
    sqlx::query("UPDATE recording_sessions SET state = 'pending' WHERE id = $1")
        .bind(session)
        .execute(&mut *transaction)
        .await?;
    transaction.rollback().await?;
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());

    // Finalizing the fragment and the session notifies; so does deleting.
    sqlx::query("UPDATE audio_files SET end_ts = 2000 WHERE id = $1")
        .bind(fragment)
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE recording_sessions SET state = 'finalized', ended_at = now() WHERE id = $1",
    )
    .bind(session)
    .execute(&pool)
    .await?;
    sqlx::query("DELETE FROM audio_files WHERE id = $1")
        .bind(fragment)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM recording_sessions WHERE id = $1")
        .bind(session)
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, vec![session_event.clone(); 4]);

    // A file without a logical session carries no identifiers.
    sqlx::query(
        "INSERT INTO audio_files (file_name, guild_id, channel_id, user_id, year, month)
         VALUES ('legacy', $1, $2, $3, 2026, 10)",
    )
    .bind(GUILD)
    .bind(CHANNEL)
    .bind(USER)
    .execute(&pool)
    .await?;
    assert_eq!(
        notifications.drain().await?,
        vec![json!({ "v": 1, "k": "recording", "g": GUILD.to_string() })]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn clips_stamps_opt_outs_and_settings_notify(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;

    sqlx::query(
        "INSERT INTO clips (clip_id, guild_id, channel_id, user_id, saved_file_name, start_time)
         VALUES ('clip-1', $1, $2, $3, 'c.ogg', 0)",
    )
    .bind(GUILD)
    .bind(CHANNEL)
    .bind(USER)
    .execute(&pool)
    .await?;
    sqlx::query("UPDATE clips SET name = name WHERE clip_id = 'clip-1'")
        .execute(&pool)
        .await?;
    let stamp: i64 = sqlx::query_scalar(
        "INSERT INTO stamps (guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts)
         VALUES ($1, $2, $3, $3, 1000) RETURNING id",
    )
    .bind(GUILD)
    .bind(CHANNEL)
    .bind(USER)
    .fetch_one(&pool)
    .await?;
    sqlx::query("INSERT INTO recording_opt_outs (guild_id, user_id) VALUES ($1, $2)")
        .bind(GUILD)
        .bind(USER)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO guild_voice_settings (guild_id, pending_cap_seconds) VALUES ($1, 120)",
    )
    .bind(GUILD)
    .execute(&pool)
    .await?;
    // Saving the same settings again only touches updated_at: silent.
    sqlx::query("UPDATE guild_voice_settings SET updated_at = now() WHERE guild_id = $1")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        vec![
            json!({
                "v": 1, "k": "clip", "g": GUILD.to_string(), "id": "clip-1",
                "c": CHANNEL.to_string(),
            }),
            json!({
                "v": 1, "k": "stamp", "g": GUILD.to_string(), "id": stamp.to_string(),
                "c": CHANNEL.to_string(),
            }),
            json!({ "v": 1, "k": "opt_out", "g": GUILD.to_string(), "u": USER.to_string() }),
            json!({
                "v": 1, "k": "settings", "g": GUILD.to_string(), "r": "guild_voice_settings",
            }),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn permission_changes_send_one_guild_payload_per_transaction(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    let guild_event = json!({ "v": 1, "k": "perm", "g": GUILD.to_string() });

    // A guild sync writing several permission rows sends one notification.
    let mut transaction = pool.begin().await?;
    sqlx::query("UPDATE roles SET permission = 1024 WHERE role_id = $1")
        .bind(ROLE)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2), (11, $2)")
        .bind(USER)
        .bind(ROLE)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 1024, 0)",
    )
    .bind(CHANNEL)
    .bind(ROLE)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    assert_eq!(notifications.drain().await?, vec![guild_event.clone()]);

    // Re-login rewrites user_guilds without changing membership: silent.
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
         VALUES ($1, $2, 'g', false, 0, ARRAY[]::text[])",
    )
    .bind(GUILD)
    .bind(USER)
    .execute(&pool)
    .await?;
    assert_eq!(notifications.drain().await?, vec![guild_event.clone()]);
    sqlx::query("UPDATE user_guilds SET name = 'renamed', icon = 'i' WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());

    // Deleting a role cascades to its assignments, whose guild can no longer
    // be resolved: a guild-less payload asks for a full resync.
    sqlx::query("DELETE FROM roles WHERE role_id = $1")
        .bind(ROLE)
        .execute(&pool)
        .await?;
    let received = notifications.drain().await?;
    assert!(received.contains(&guild_event), "{received:?}");
    assert!(
        received.contains(&json!({ "v": 1, "k": "perm" })),
        "{received:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn presence_and_roster_changes_notify_without_identifiers(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    let presence = json!({ "v": 1, "k": "presence", "g": GUILD.to_string() });
    let members = json!({ "v": 1, "k": "members", "g": GUILD.to_string() });
    // Membership decides access, so joins, leaves and a roster becoming
    // complete also re-authorize the guild's sockets.
    let perm = json!({ "v": 1, "k": "perm", "g": GUILD.to_string() });

    // A claim makes both known, and the roster complete.
    sqlx::query(
        "INSERT INTO guild_projection_state
            (guild_id, owner_instance_id, generation, roster_complete_at, presence_synced_at)
         VALUES ($1, 'bot', 1, now(), now())",
    )
    .bind(GUILD)
    .execute(&pool)
    .await?;
    assert_eq!(
        notifications.drain().await?,
        [members.clone(), presence.clone(), perm.clone()]
    );

    // Joining, muting and leaving voice: one guild-only payload per
    // transaction, never a channel or a user.
    let mut transaction = pool.begin().await?;
    sqlx::query("INSERT INTO guild_members (guild_id, user_id, username) VALUES ($1, $2, 'user')")
        .bind(GUILD)
        .bind(USER)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO voice_presence (guild_id, user_id, channel_id) VALUES ($1, $2, $3)")
        .bind(GUILD)
        .bind(USER)
        .bind(CHANNEL)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    assert_eq!(
        notifications.drain().await?,
        [members.clone(), perm.clone(), presence.clone()]
    );
    sqlx::query("UPDATE voice_presence SET self_mute = true WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        std::slice::from_ref(&presence)
    );

    // A rename of someone in voice changes what presence shows; a no-op
    // write changes nothing.
    sqlx::query("UPDATE guild_members SET nickname = 'nick' WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        [members.clone(), presence.clone()]
    );
    sqlx::query("UPDATE guild_members SET nickname = 'nick' WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE voice_presence SET self_mute = true WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    // A refreshed sync time or a re-claim by the same owner shows nothing new.
    sqlx::query(
        "UPDATE guild_projection_state
            SET generation = generation + 1, presence_synced_at = now(), roster_complete_at = now()",
    )
    .execute(&pool)
    .await?;
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());

    sqlx::query("DELETE FROM voice_presence WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        std::slice::from_ref(&presence)
    );

    // The owner stopping makes both unknown; the complete roster stays
    // authoritative, so access does not change.
    sqlx::query("UPDATE guild_projection_state SET owner_instance_id = NULL")
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, [members, presence]);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn concurrent_recording_writes_all_commit_and_notify(pool: PgPool) -> TestResult {
    // NOTIFY serializes notifying commits on a global lock; many recorders
    // writing at once must still all commit, and every write must arrive.
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    const WRITERS: usize = 16;
    const ROUNDS: usize = 10;

    let mut tasks = Vec::new();
    for writer in 0..WRITERS {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            for round in 0..ROUNDS {
                let mut transaction = pool.begin().await?;
                let session: i64 = sqlx::query_scalar(
                    "INSERT INTO recording_sessions
                        (guild_id, user_id, starting_channel_id, current_channel_id, state, started_at)
                     VALUES ($1, $2, $3, $3, 'active', now())
                     RETURNING id",
                )
                .bind(GUILD)
                .bind(USER)
                .bind(CHANNEL)
                .fetch_one(&mut *transaction)
                .await?;
                let fragment: i64 = sqlx::query_scalar(
                    "INSERT INTO audio_files
                        (file_name, guild_id, channel_id, user_id, year, month, start_ts,
                         recording_session_id, segment_index)
                     VALUES ($1, $2, $3, $4, 2026, 10, 1000, $5, 0)
                     RETURNING id",
                )
                .bind(format!("w{writer}-r{round}"))
                .bind(GUILD)
                .bind(CHANNEL)
                .bind(USER)
                .bind(session)
                .fetch_one(&mut *transaction)
                .await?;
                transaction.commit().await?;
                sqlx::query("UPDATE audio_files SET recording_heartbeat_at = now() WHERE id = $1")
                    .bind(fragment)
                    .execute(&pool)
                    .await?;
            }
            Ok::<_, sqlx::Error>(())
        }));
    }
    for task in tasks {
        task.await??;
    }

    // Each transaction sent one payload (session and fragment are identical
    // within it); heartbeats sent none.
    assert_eq!(notifications.drain().await?.len(), WRITERS * ROUNDS);
    Ok(())
}

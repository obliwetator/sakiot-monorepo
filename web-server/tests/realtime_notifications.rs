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

    // The recorder heartbeat (every 10 s) and session bookkeeping are silent.
    sqlx::query(
        "INSERT INTO recorder_heartbeats (recorder_id, owner_instance_id, guild_id)
         VALUES ('recorder', 'bot', $1)
         ON CONFLICT (recorder_id) DO UPDATE SET heartbeat_at = now()",
    )
    .bind(GUILD)
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
        "INSERT INTO clips (clip_id, guild_id, channel_id, channel_ids, speaker_ids, user_id, saved_file_name, start_time)
         VALUES ('clip-1', $1, $2, ARRAY[$2::bigint], ARRAY[$3::bigint], $3, 'c.ogg', 0)",
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
                "cs": [CHANNEL.to_string()], "ss": [USER.to_string()],
            }),
            json!({
                "v": 1, "k": "stamp", "g": GUILD.to_string(), "id": stamp.to_string(),
                "c": CHANNEL.to_string(), "u": USER.to_string(),
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
async fn a_clip_whose_channels_or_speakers_change_notifies_with_the_old_and_the_new(
    pool: PgPool,
) -> TestResult {
    const OTHER_CHANNEL: i64 = 101;
    const OTHER_USER: i64 = 12;
    seed(&pool).await?;
    sqlx::query(
        "INSERT INTO clips (clip_id, guild_id, channel_id, channel_ids, speaker_ids, user_id, saved_file_name, start_time, original_file_name)
         VALUES ('composed', $1, $2, ARRAY[$2::bigint], ARRAY[$3::bigint], $3, 'c.ogg', 0, 'compose')",
    )
    .bind(GUILD)
    .bind(CHANNEL)
    .bind(USER)
    .execute(&pool)
    .await?;
    let mut notifications = Notifications::listen(&pool).await?;

    // An overwritten composition: whoever could hear the old one hears that
    // it changed, and so does whoever can hear the new one.
    sqlx::query("UPDATE clips SET channel_ids = $1 WHERE clip_id = 'composed'")
        .bind([CHANNEL, OTHER_CHANNEL])
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE clips SET speaker_ids = $1 WHERE clip_id = 'composed'")
        .bind([USER, OTHER_USER])
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM clips WHERE clip_id = 'composed'")
        .execute(&pool)
        .await?;
    let clip = |channels: &[i64], speakers: &[i64]| {
        json!({
            "v": 1, "k": "clip", "g": GUILD.to_string(), "id": "composed",
            "cs": channels.iter().map(i64::to_string).collect::<Vec<_>>(),
            "ss": speakers.iter().map(i64::to_string).collect::<Vec<_>>(),
        })
    };
    assert_eq!(
        notifications.drain().await?,
        vec![
            clip(&[CHANNEL], &[USER]),
            clip(&[CHANNEL, OTHER_CHANNEL], &[USER]),
            clip(&[CHANNEL, OTHER_CHANNEL], &[USER]),
            clip(&[CHANNEL, OTHER_CHANNEL], &[USER, OTHER_USER]),
            clip(&[CHANNEL, OTHER_CHANNEL], &[USER, OTHER_USER]),
        ]
    );
    Ok(())
}

/// The bot leaving a guild makes everyone there a former member, and its
/// return brings them back: either re-authorizes the guild's viewers.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn the_bot_leaving_or_returning_reauthorizes_the_guild(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    let guild_event = json!({ "v": 1, "k": "perm", "g": GUILD.to_string() });

    sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1)")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    // Seen again at startup: nothing changed, nothing sent.
    sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM guilds_present WHERE guild_id = $1")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        vec![guild_event.clone(), guild_event]
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
    // The role list counts the new assignments.
    let members = json!({ "v": 1, "k": "members", "g": GUILD.to_string() });
    assert_eq!(
        notifications.drain().await?,
        vec![guild_event.clone(), members]
    );

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
    // be resolved: a guild-less payload asks for a full resync. The role list
    // loses the role.
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
    assert!(
        received.contains(&json!({ "v": 1, "k": "names", "g": GUILD.to_string(), "r": "roles" })),
        "{received:?}"
    );
    Ok(())
}

/// Runs `sql` and checks what it notified, in any order: triggers on one
/// row fire by name.
async fn expect(
    pool: &PgPool,
    notifications: &mut Notifications,
    sql: &'static str,
    expected: Vec<Value>,
) -> TestResult {
    sqlx::query(sql).execute(pool).await?;
    assert_eq!(
        unordered(notifications.drain().await?),
        unordered(expected),
        "{sql}"
    );
    Ok(())
}

/// The same payloads in any order.
fn unordered(mut payloads: Vec<Value>) -> Vec<String> {
    let mut payloads: Vec<String> = payloads
        .drain(..)
        .map(|payload| payload.to_string())
        .collect();
    payloads.sort();
    payloads
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn renames_send_names_and_only_authorization_sends_perm(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    let perm = json!({ "v": 1, "k": "perm", "g": GUILD.to_string() });
    let names = |table: &str| json!({ "v": 1, "k": "names", "g": GUILD.to_string(), "r": table });
    let channel = |channel_id: i64| {
        json!({
            "v": 1, "k": "names", "g": GUILD.to_string(), "r": "channels",
            "c": channel_id.to_string(),
        })
    };

    // The guild: its name and icon show, its owner decides.
    expect(
        &pool,
        &mut notifications,
        "UPDATE guilds SET name = 'Guild', icon = 'i'",
        vec![names("guilds")],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "UPDATE guilds SET owner_id = 21",
        vec![perm.clone()],
    )
    .await?;

    // Roles: names and colours show, and the role list shows every role;
    // the permission bits decide.
    expect(
        &pool,
        &mut notifications,
        "UPDATE roles SET name = 'Renamed' WHERE role_id = 1001",
        vec![names("roles")],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "UPDATE roles SET color = 5 WHERE role_id = 1001",
        vec![names("roles")],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "UPDATE roles SET permission = 8 WHERE role_id = 1001",
        vec![perm.clone()],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "INSERT INTO roles (guild_id, role_id, permission, name) VALUES (1, 1002, 0, 'New')",
        vec![perm.clone(), names("roles")],
    )
    .await?;

    // Channels: only voice and stage channels count or show anywhere.
    expect(
        &pool,
        &mut notifications,
        "UPDATE channels SET name = 'Renamed' WHERE channel_id = 100",
        vec![channel(100)],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "INSERT INTO channels (channel_id, guild_id, type, name) VALUES (101, 1, 0, 'text')",
        vec![perm.clone()],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "UPDATE channels SET name = 'chat' WHERE channel_id = 101",
        vec![],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "INSERT INTO channels (channel_id, guild_id, type, name) VALUES (102, 1, 2, 'voice 2')",
        vec![perm.clone(), channel(102)],
    )
    .await?;
    expect(
        &pool,
        &mut notifications,
        "UPDATE channels SET type = 0 WHERE channel_id = 102",
        vec![perm.clone(), channel(102)],
    )
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn presence_changes_name_the_member_and_the_channel_they_left(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    // Without a member: refetch the whole list.
    let presence = json!({ "v": 1, "k": "presence", "g": GUILD.to_string() });
    let joined = json!({
        "v": 1, "k": "presence", "g": GUILD.to_string(), "u": USER.to_string(),
    });
    let changed_in = |channel: i64| {
        json!({
            "v": 1, "k": "presence", "g": GUILD.to_string(), "u": USER.to_string(),
            "o": channel.to_string(),
        })
    };
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

    // Joining, muting, moving and leaving voice name the member and, after
    // joining, the channel they were in. Where they are now is read when the
    // change is routed, so no payload says where they went.
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
        [members.clone(), perm.clone(), joined]
    );
    sqlx::query("UPDATE voice_presence SET self_mute = true WHERE user_id = $1")
        .bind(USER)
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, [changed_in(CHANNEL)]);
    sqlx::query("UPDATE voice_presence SET channel_id = $2 WHERE user_id = $1")
        .bind(USER)
        .bind(CHANNEL + 1)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE voice_presence SET channel_id = $2 WHERE user_id = $1")
        .bind(USER)
        .bind(CHANNEL)
        .execute(&pool)
        .await?;
    assert_eq!(
        notifications.drain().await?,
        [changed_in(CHANNEL), changed_in(CHANNEL + 1)]
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
    assert_eq!(notifications.drain().await?, [changed_in(CHANNEL)]);

    // The owner stopping makes both unknown; the complete roster stays
    // authoritative, so access does not change.
    sqlx::query("UPDATE guild_projection_state SET owner_instance_id = NULL")
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, [members, presence]);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn jobs_notify_what_their_status_shows(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut notifications = Notifications::listen(&pool).await?;
    sqlx::query(
        "INSERT INTO media_jobs (id, kind, guild_id, user_id, idempotency_key, resource_key, request)
         VALUES ('m1', 'session_waveform', $1, $2, 'k', 'r', '{}')",
    )
    .bind(GUILD)
    .bind(USER)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO composition_jobs
            (id, guild_id, user_id, idempotency_key, request, snapshot, result_clip_id)
         VALUES ('c1', $1, $2, 'k', '{}', '{}', 'clip')",
    )
    .bind(GUILD)
    .bind(USER)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO recording_deletion_jobs (id, recording_session_id, guild_id, reason, mode)
         VALUES ('d1', 5, $1, 'manager', 'permanent')",
    )
    .bind(GUILD)
    .execute(&pool)
    .await?;
    // Whoever creates or joins a job reads its status in that request.
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());

    sqlx::query("UPDATE media_jobs SET progress = 40 WHERE id = 'm1'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE composition_jobs SET stage = 'rendering' WHERE id = 'c1'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE recording_deletion_jobs SET state = 'failed', stage = 'failed' WHERE id = 'd1'",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        notifications.drain().await?,
        vec![
            json!({ "v": 1, "k": "job", "r": "media", "g": GUILD.to_string(), "id": "m1" }),
            json!({
                "v": 1, "k": "job", "r": "composition", "g": GUILD.to_string(), "id": "c1",
                "u": USER.to_string(),
            }),
            json!({ "v": 1, "k": "job", "r": "deletion", "g": GUILD.to_string(), "id": "d1" }),
        ]
    );

    // Unchanged progress and bookkeeping (retry times, timestamps) are silent.
    sqlx::query("UPDATE media_jobs SET progress = 40, retry_at = now(), updated_at = now()")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE composition_jobs SET updated_at = now()")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE recording_deletion_jobs SET attempts = 1, updated_at = now()")
        .execute(&pool)
        .await?;
    assert_eq!(notifications.drain().await?, Vec::<Value>::new());
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
                sqlx::query(
                    "INSERT INTO audio_files
                        (file_name, guild_id, channel_id, user_id, year, month, start_ts,
                         recording_session_id, segment_index)
                     VALUES ($1, $2, $3, $4, 2026, 10, 1000, $5, 0)",
                )
                .bind(format!("w{writer}-r{round}"))
                .bind(GUILD)
                .bind(CHANNEL)
                .bind(USER)
                .bind(session)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                sqlx::query(
                    "INSERT INTO recorder_heartbeats (recorder_id, owner_instance_id, guild_id)
                     VALUES ($1, 'bot', $2)
                     ON CONFLICT (recorder_id) DO UPDATE SET heartbeat_at = now()",
                )
                .bind(format!("recorder-{writer}"))
                .bind(GUILD)
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

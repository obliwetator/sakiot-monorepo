//! Member and presence projections: complete snapshots only, fenced writers,
//! and handover between instances.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serenity::all::{ChannelId, Guild, GuildId, Member, User, UserId, VoiceState};
use serenity::cache::Cache;
use serenity::model::event::{GuildCreateEvent, GuildMemberRemoveEvent, VoiceStateUpdateEvent};
use sqlx::PgPool;

use crate::database::projections as db;
use crate::projections::{Projections, Refresh};
use crate::runtime::{BotRole, RuntimeConfig, RuntimeState};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn base() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() % 1_000_000_000)
        .unwrap_or_default();
    u64::try_from(millis).unwrap_or_default() * 100 + 9_000_000_000_000
}

/// Gateway events are `#[non_exhaustive]`; build them the way the gateway
/// does, from JSON.
fn event<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
}

fn voice_event(state: VoiceState) -> Result<VoiceStateUpdateEvent, serde_json::Error> {
    event(serde_json::to_value(state)?)
}

fn id(value: u64) -> i64 {
    i64::try_from(value).unwrap_or_default()
}

fn runtime(instance_id: &str, role: BotRole) -> Arc<RuntimeState> {
    RuntimeState::new(RuntimeConfig {
        instance_id: instance_id.to_string(),
        grpc_address: None,
        initial_role: role,
        drain_timeout: Duration::from_secs(30),
    })
}

fn member(guild_id: GuildId, user_id: UserId, name: &str) -> Member {
    let mut member = Member::default();
    member.guild_id = guild_id;
    let mut user = User::default();
    user.id = user_id;
    user.name = name.to_string();
    member.user = user;
    member
}

fn voice(guild_id: GuildId, user_id: UserId, channel: Option<ChannelId>) -> VoiceState {
    serde_json::from_value(serde_json::json!({
        "guild_id": guild_id,
        "user_id": user_id,
        "channel_id": channel,
        "session_id": "session",
        "deaf": false,
        "mute": false,
        "self_deaf": false,
        "self_mute": true,
        "self_video": false,
        "suppress": false,
        "request_to_speak_timestamp": null,
    }))
    .unwrap_or_else(|err| panic!("voice state fixture: {err}"))
}

struct Fixture {
    guild_id: GuildId,
    channel: ChannelId,
    users: [UserId; 3],
}

impl Fixture {
    fn new() -> Self {
        let base = base();
        Self {
            guild_id: GuildId::new(base),
            channel: ChannelId::new(base + 1),
            users: [
                UserId::new(base + 10),
                UserId::new(base + 11),
                UserId::new(base + 12),
            ],
        }
    }

    /// The guild with `cached` of its three members in the member list, the
    /// first of them in voice.
    fn guild(&self, cached: usize) -> Guild {
        let mut guild = Guild::default();
        guild.id = self.guild_id;
        guild.member_count = 3;
        for (index, user_id) in self.users.iter().take(cached).enumerate() {
            guild.members.insert(
                *user_id,
                member(self.guild_id, *user_id, &format!("user-{index}")),
            );
        }
        let [first, ..] = self.users;
        guild
            .voice_states
            .insert(first, voice(self.guild_id, first, Some(self.channel)));
        guild
    }

    /// A cache holding the guild, as GUILD_CREATE leaves it.
    fn cache(&self, cached: usize) -> Result<Cache, serde_json::Error> {
        let cache = Cache::new();
        let mut created: GuildCreateEvent = event(serde_json::to_value(self.guild(cached))?)?;
        cache.update(&mut created);
        Ok(cache)
    }

    fn guild_i64(&self) -> i64 {
        id(self.guild_id.get())
    }
}

async fn roster(pool: &PgPool, guild_id: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar("SELECT user_id FROM guild_members WHERE guild_id = $1 ORDER BY user_id")
        .bind(guild_id)
        .fetch_all(pool)
        .await
}

async fn in_voice(pool: &PgPool, guild_id: i64) -> sqlx::Result<Vec<(i64, i64)>> {
    sqlx::query_as(
        "SELECT user_id, channel_id FROM voice_presence WHERE guild_id = $1 ORDER BY user_id",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

async fn owner(pool: &PgPool, guild_id: i64) -> sqlx::Result<Option<(Option<String>, i64)>> {
    sqlx::query_as(
        "SELECT owner_instance_id, generation FROM guild_projection_state WHERE guild_id = $1",
    )
    .bind(guild_id)
    .fetch_optional(pool)
    .await
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_incomplete_roster_is_never_claimed_nor_pruned(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let guild = fixture.guild_i64();
    let (projections, _requests) = Projections::new(pool.clone(), runtime("a", BotRole::Active));

    // GUILD_CREATE for a large guild holds part of the roster: no claim, so
    // presence stays unknown until the chunks arrive.
    let partial = fixture.cache(2)?;
    assert_eq!(
        projections.refresh(&partial, fixture.guild_id).await?,
        Refresh::NeedsChunks
    );
    assert_eq!(owner(&pool, guild).await?, None);

    let complete = fixture.cache(3)?;
    assert_eq!(
        projections.refresh(&complete, fixture.guild_id).await?,
        Refresh::Done
    );
    let users: Vec<i64> = fixture.users.iter().map(|user| id(user.get())).collect();
    assert_eq!(roster(&pool, guild).await?, users);
    assert_eq!(
        in_voice(&pool, guild).await?,
        vec![(users[0], id(fixture.channel.get()))]
    );

    // The owner refreshing from a partial cache (after a re-identify) keeps
    // the members it cannot see and asks for chunks again.
    assert_eq!(
        projections.refresh(&partial, fixture.guild_id).await?,
        Refresh::NeedsChunks
    );
    assert_eq!(roster(&pool, guild).await?, users);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_replacement_claim_fences_the_previous_owner(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let guild = fixture.guild_i64();
    let [first, second, _] = fixture.users;
    let cache = fixture.cache(3)?;
    let (old, _old_requests) = Projections::new(pool.clone(), runtime("old", BotRole::Active));
    let (new, _new_requests) = Projections::new(pool.clone(), runtime("new", BotRole::Active));

    old.refresh(&cache, fixture.guild_id).await?;
    assert_eq!(owner(&pool, guild).await?, Some((Some("old".into()), 1)));

    // The old release starts draining: it keeps writing what it owns.
    let mut joined = voice_event(voice(fixture.guild_id, second, Some(fixture.channel)))?;
    cache.update(&mut joined);
    old.user_changed(&cache, fixture.guild_id, second).await;
    assert_eq!(in_voice(&pool, guild).await?.len(), 2);

    // The new release claims with its complete snapshot.
    new.refresh(&cache, fixture.guild_id).await?;
    assert_eq!(owner(&pool, guild).await?, Some((Some("new".into()), 2)));

    // The old instance's later writes are fenced out, even when its cache
    // says otherwise.
    let mut left = voice_event(voice(fixture.guild_id, first, None))?;
    cache.update(&mut left);
    old.user_changed(&cache, fixture.guild_id, first).await;
    assert_eq!(in_voice(&pool, guild).await?.len(), 2);
    new.user_changed(&cache, fixture.guild_id, first).await;
    assert_eq!(
        in_voice(&pool, guild).await?,
        vec![(id(second.get()), id(fixture.channel.get()))]
    );

    // Stopping releases only what an instance still owns.
    assert_eq!(db::release_all(&pool, "old").await?, 0);
    assert_eq!(db::release_all(&pool, "new").await?, 1);
    assert_eq!(owner(&pool, guild).await?, Some((None, 2)));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_draining_instance_never_claims(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let cache = fixture.cache(3)?;
    let (draining, _requests) = Projections::new(pool.clone(), runtime("old", BotRole::Drain));
    assert_eq!(
        draining.refresh(&cache, fixture.guild_id).await?,
        Refresh::Done
    );
    assert_eq!(owner(&pool, fixture.guild_i64()).await?, None);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn only_a_member_remove_event_drops_a_member(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let guild = fixture.guild_i64();
    let [first, second, third] = fixture.users;
    let cache = fixture.cache(3)?;
    let (projections, _requests) = Projections::new(pool.clone(), runtime("a", BotRole::Active));
    projections.refresh(&cache, fixture.guild_id).await?;

    // Absent from the cache is not gone: a user write keeps the row.
    let other_cache = fixture.cache(1)?;
    projections
        .user_changed(&other_cache, fixture.guild_id, third)
        .await;
    assert_eq!(roster(&pool, guild).await?.len(), 3);

    let user = member(fixture.guild_id, first, "user-0").user;
    let mut removed: GuildMemberRemoveEvent = event(serde_json::json!({
        "guild_id": fixture.guild_id,
        "user": user,
    }))?;
    cache.update(&mut removed);
    projections
        .member_left(&cache, fixture.guild_id, first)
        .await;
    assert_eq!(
        roster(&pool, guild).await?,
        vec![id(second.get()), id(third.get())]
    );
    // Their voice state went with them.
    assert!(in_voice(&pool, guild).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_unchanged_refresh_writes_no_rows(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let guild = fixture.guild_i64();
    let cache = fixture.cache(3)?;
    let (projections, _requests) = Projections::new(pool.clone(), runtime("a", BotRole::Active));
    projections.refresh(&cache, fixture.guild_id).await?;

    let versions = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT string_agg(xmin::text, ',' ORDER BY user_id) FROM (
                 SELECT user_id, xmin FROM guild_members WHERE guild_id = $1
                 UNION ALL
                 SELECT user_id, xmin FROM voice_presence WHERE guild_id = $1) rows",
        )
        .bind(guild)
        .fetch_one(&pool)
        .await
    };
    let before = versions().await?;
    projections.refresh(&cache, fixture.guild_id).await?;
    projections
        .user_changed(&cache, fixture.guild_id, fixture.users[0])
        .await;
    assert_eq!(versions().await?, before);
    Ok(())
}

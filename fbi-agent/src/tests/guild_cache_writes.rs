//! Guild cache writers commit one guild at a time and write only real
//! differences, so readers never see a half-rewritten guild and a resync that
//! finds nothing new writes no rows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serenity::all::{
    ChannelId, ChannelType, Guild, GuildChannel, GuildId, Member, PermissionOverwrite,
    PermissionOverwriteType, Permissions, Role, RoleId, User, UserId,
};
use sqlx::PgPool;

use crate::database::guild_cache;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    guild_id: GuildId,
    owner_id: UserId,
    roles: [RoleId; 2],
    channels: [ChannelId; 2],
    members: [UserId; 3],
}

impl Fixture {
    fn new() -> Self {
        let base = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() % 1_000_000_000)
            .unwrap_or_default();
        let base = u64::try_from(base).unwrap_or_default() * 100 + 8_000_000_000_000;
        Self {
            guild_id: GuildId::new(base),
            owner_id: UserId::new(base + 1),
            roles: [RoleId::new(base + 10), RoleId::new(base + 11)],
            channels: [ChannelId::new(base + 20), ChannelId::new(base + 21)],
            members: [
                UserId::new(base + 30),
                UserId::new(base + 31),
                UserId::new(base + 32),
            ],
        }
    }

    fn role(&self, role_id: RoleId, name: &str) -> Role {
        let mut role = Role::default();
        role.id = role_id;
        role.guild_id = self.guild_id;
        role.name = name.to_string();
        role.permissions = Permissions::VIEW_CHANNEL;
        role
    }

    fn channel(&self, channel_id: ChannelId, overwrites: Vec<PermissionOverwrite>) -> GuildChannel {
        let mut channel = GuildChannel::default();
        channel.id = channel_id;
        channel.guild_id = self.guild_id;
        channel.kind = ChannelType::Voice;
        channel.name = format!("voice-{channel_id}");
        channel.permission_overwrites = overwrites;
        channel
    }

    fn member(&self, user_id: UserId, roles: Vec<RoleId>) -> Member {
        let mut member = Member::default();
        member.guild_id = self.guild_id;
        let mut user = User::default();
        user.id = user_id;
        member.user = user;
        member.roles = roles;
        member
    }

    fn role_overwrite(&self, role_id: RoleId, allow: Permissions) -> PermissionOverwrite {
        PermissionOverwrite {
            allow,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Role(role_id),
        }
    }

    fn member_overwrite(&self, user_id: UserId, deny: Permissions) -> PermissionOverwrite {
        PermissionOverwrite {
            allow: Permissions::empty(),
            deny,
            kind: PermissionOverwriteType::Member(user_id),
        }
    }

    /// Two roles, two channels with role and member overwrites, and three
    /// members holding roles; the cached member list is complete.
    fn guild(&self) -> Guild {
        let [first_role, second_role] = self.roles;
        let [first_channel, second_channel] = self.channels;
        let [alice, bob, carol] = self.members;

        let mut guild = Guild::default();
        guild.id = self.guild_id;
        guild.owner_id = self.owner_id;
        guild
            .roles
            .insert(first_role, self.role(first_role, "first"));
        guild
            .roles
            .insert(second_role, self.role(second_role, "second"));
        guild.channels.insert(
            first_channel,
            self.channel(
                first_channel,
                vec![
                    self.role_overwrite(first_role, Permissions::CONNECT),
                    self.member_overwrite(carol, Permissions::CONNECT),
                ],
            ),
        );
        guild.channels.insert(
            second_channel,
            self.channel(
                second_channel,
                vec![self.role_overwrite(second_role, Permissions::VIEW_CHANNEL)],
            ),
        );
        guild
            .members
            .insert(alice, self.member(alice, vec![first_role, second_role]));
        guild
            .members
            .insert(bob, self.member(bob, vec![first_role]));
        guild.members.insert(carol, self.member(carol, Vec::new()));
        guild.member_count = 3;
        guild
    }

    fn ids(&self) -> (i64, Vec<i64>) {
        let guild_id = i64::try_from(self.guild_id.get()).unwrap_or_default();
        let roles = self
            .roles
            .iter()
            .filter_map(|role| i64::try_from(role.get()).ok())
            .collect();
        (guild_id, roles)
    }
}

fn id(value: u64) -> i64 {
    i64::try_from(value).unwrap_or_default()
}

/// Every cache row of the guild with the transaction id that last wrote it.
/// Any write, even one storing identical values, changes `xmin`.
async fn row_versions(pool: &PgPool, fixture: &Fixture) -> sqlx::Result<Vec<(String, String)>> {
    let (guild_id, _) = fixture.ids();
    sqlx::query_as(
        "SELECT 'guilds:' || id, xmin::text FROM guilds WHERE id = $1
         UNION ALL
         SELECT 'user_guilds:' || user_id, xmin::text FROM user_guilds WHERE id = $1
         UNION ALL
         SELECT 'roles:' || role_id, xmin::text FROM roles WHERE guild_id = $1
         UNION ALL
         SELECT 'channels:' || channel_id, xmin::text FROM channels WHERE guild_id = $1
         UNION ALL
         SELECT 'channel_permissions:' || cp.channel_id || ':' || cp.target_id, cp.xmin::text
           FROM channel_permissions cp
           JOIN channels c ON c.channel_id = cp.channel_id
          WHERE c.guild_id = $1
         UNION ALL
         SELECT 'user_roles:' || ur.user_id || ':' || ur.role_id, ur.xmin::text
           FROM user_roles ur
           JOIN roles r ON r.role_id = ur.role_id
          WHERE r.guild_id = $1
         ORDER BY 1",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

async fn seed_oauth_rows(pool: &PgPool, fixture: &Fixture) -> sqlx::Result<()> {
    let (guild_id, _) = fixture.ids();
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
         VALUES ($1, $2, 'cache-writes', true, 0, ARRAY[]::text[]),
                ($1, $3, 'cache-writes', false, 0, ARRAY[]::text[])",
    )
    .bind(guild_id)
    .bind(id(fixture.owner_id.get()))
    .bind(id(fixture.members[0].get()))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn unchanged_cache_writes_touch_no_rows(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let guild = fixture.guild();
    guild_cache::sync_new_guild(&pool, &guild).await?;
    seed_oauth_rows(&pool, &fixture).await?;
    let before = row_versions(&pool, &fixture).await?;
    // guild, two user_guilds rows, two roles, two channels, three overwrites,
    // three member role assignments.
    assert_eq!(before.len(), 13);

    // A full resync and every live writer, each fed what is already cached.
    guild_cache::sync_new_guild(&pool, &guild).await?;
    for role in guild.roles.values() {
        guild_cache::sync_live_role(&pool, role).await?;
    }
    for channel in guild.channels.values() {
        guild_cache::sync_live_channel(&pool, channel).await?;
    }
    for member in guild.members.values() {
        guild_cache::sync_live_member_roles(&pool, guild.id, member.user.id, &member.roles).await?;
    }
    guild_cache::sync_guild_info(&pool, &guild.clone().into()).await?;

    assert_eq!(row_versions(&pool, &fixture).await?, before);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn cache_writes_touch_only_changed_rows(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let mut guild = fixture.guild();
    guild_cache::sync_new_guild(&pool, &guild).await?;
    let before = row_versions(&pool, &fixture).await?;

    // One overwrite changes its bits and the second channel's overwrite goes
    // away; one member gains a role.
    let [first_channel, second_channel] = fixture.channels;
    let [first_role, second_role] = fixture.roles;
    let [_, bob, carol] = fixture.members;
    if let Some(channel) = guild.channels.get_mut(&first_channel) {
        channel.permission_overwrites = vec![
            fixture.role_overwrite(first_role, Permissions::CONNECT | Permissions::SPEAK),
            fixture.member_overwrite(carol, Permissions::CONNECT),
        ];
    }
    if let Some(channel) = guild.channels.get_mut(&second_channel) {
        channel.permission_overwrites = Vec::new();
    }
    if let Some(member) = guild.members.get_mut(&bob) {
        member.roles = vec![first_role, second_role];
    }
    guild_cache::sync_new_guild(&pool, &guild).await?;

    let after = row_versions(&pool, &fixture).await?;
    let changed: Vec<&str> = after
        .iter()
        .filter(|row| !before.contains(row))
        .map(|(key, _)| key.as_str())
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .filter(|(key, _)| !after.iter().any(|(after_key, _)| after_key == key))
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(
        changed,
        vec![
            format!("channel_permissions:{first_channel}:{first_role}"),
            format!("user_roles:{bob}:{second_role}"),
        ]
    );
    assert_eq!(
        removed,
        vec![format!(
            "channel_permissions:{second_channel}:{second_role}"
        )]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn incomplete_member_list_keeps_absent_members_roles(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let mut guild = fixture.guild();
    guild_cache::sync_new_guild(&pool, &guild).await?;
    let [first_role, _] = fixture.roles;
    let [alice, bob, _] = fixture.members;

    // Discord sent only part of a large guild's members: Bob is missing from
    // the cache, and Alice lost a role.
    guild.member_count = 300;
    guild.members.remove(&bob);
    if let Some(member) = guild.members.get_mut(&alice) {
        member.roles = vec![first_role];
    }
    guild_cache::sync_new_guild(&pool, &guild).await?;

    let assignments = |pool: PgPool| async move {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT ur.user_id, ur.role_id
               FROM user_roles ur
               JOIN roles r ON r.role_id = ur.role_id
              WHERE r.guild_id = $1
              ORDER BY 1, 2",
        )
        .bind(id(fixture.guild_id.get()))
        .fetch_all(&pool)
        .await
    };
    assert_eq!(
        assignments(pool.clone()).await?,
        vec![
            (id(alice.get()), id(first_role.get())),
            (id(bob.get()), id(first_role.get()))
        ],
        "a cached member is corrected; an absent one keeps their roles"
    );

    // Once the cache holds every member, absent members are pruned.
    guild.member_count = 2;
    guild_cache::sync_new_guild(&pool, &guild).await?;
    assert_eq!(
        assignments(pool.clone()).await?,
        vec![(id(alice.get()), id(first_role.get()))]
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn readers_never_see_a_partly_rewritten_guild(pool: PgPool) -> TestResult {
    let fixture = Fixture::new();
    let mut guild = fixture.guild();
    guild_cache::sync_new_guild(&pool, &guild).await?;
    let (guild_id, _) = fixture.ids();
    let [first_channel, _] = fixture.channels;
    let [first_role, second_role] = fixture.roles;
    let [_, _, carol] = fixture.members;

    // Resyncs alternate between two states that share the baseline rows, so
    // any reader seeing fewer than the baseline caught a half-written guild.
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let pool = pool.clone();
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            let mut reads = 0_u32;
            let mut low = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                let (overwrites, assignments): (i64, i64) = sqlx::query_as(
                    "SELECT
                        (SELECT count(*) FROM channel_permissions cp
                           JOIN channels c ON c.channel_id = cp.channel_id
                          WHERE c.guild_id = $1),
                        (SELECT count(*) FROM user_roles ur
                           JOIN roles r ON r.role_id = ur.role_id
                          WHERE r.guild_id = $1)",
                )
                .bind(guild_id)
                .fetch_one(&pool)
                .await?;
                reads += 1;
                if overwrites < 3 || assignments < 3 {
                    low.push((overwrites, assignments));
                }
            }
            Ok::<_, sqlx::Error>((reads, low))
        })
    };

    for round in 0..40 {
        let extra = round % 2 == 0;
        if let Some(channel) = guild.channels.get_mut(&first_channel) {
            channel.permission_overwrites = vec![
                fixture.role_overwrite(first_role, Permissions::CONNECT),
                fixture.member_overwrite(carol, Permissions::CONNECT),
            ];
            if extra {
                channel
                    .permission_overwrites
                    .push(fixture.role_overwrite(second_role, Permissions::SPEAK));
            }
        }
        if let Some(member) = guild.members.get_mut(&carol) {
            member.roles = if extra { vec![second_role] } else { Vec::new() };
        }
        guild_cache::sync_new_guild(&pool, &guild).await?;
    }
    stop.store(true, Ordering::SeqCst);

    let (reads, low) = reader.await??;
    assert!(reads > 0, "the reader ran");
    assert!(low.is_empty(), "partial reads: {low:?}");
    Ok(())
}

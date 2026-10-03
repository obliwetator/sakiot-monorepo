use crate::cast::ToI64;
use crate::database::DbResult;
use crate::event_handler::Handler;
use serenity::{
    all::UnavailableGuild,
    model::prelude::{Guild, GuildChannel, GuildId, PermissionOverwriteType, Role, RoleId, UserId},
    prelude::Context,
};
use sqlx::{PgConnection, Pool, Postgres};
use tracing::error;

pub(crate) async fn update_info(handler: &Handler, ctx: &Context, guilds: &[GuildId]) {
    let guild_cached: Vec<Guild> = guilds
        .iter()
        .filter_map(|guild| {
            guild
                .to_guild_cached(ctx)
                .map(|g| g.to_owned())
                .or_else(|| {
                    error!("Guild {} missing from cache during database update", guild);
                    None
                })
        })
        .collect();

    if let Err(err) = sync_info(&handler.database, &guild_cached).await {
        error!(error = %err, "failed to sync guild cache");
    }
}

async fn sync_info(pool: &Pool<Postgres>, guild_cached: &[Guild]) -> DbResult<()> {
    for guild in guild_cached {
        sync_guild(pool, guild).await?;
    }
    Ok(())
}

/// One guild's full cache sync, in one transaction that writes only real
/// differences. Readers see the old or the new cache state, never a guild
/// with its role assignments or channel overwrites half rewritten, and a
/// resync that finds nothing new writes no rows.
async fn sync_guild(pool: &Pool<Postgres>, guild: &Guild) -> DbResult<()> {
    let guild_id = guild.id.to_i64();
    let mut transaction = pool.begin().await?;

    upsert_guild_owner(&mut transaction, guild_id, guild.owner_id.to_i64()).await?;

    let roles: Vec<&Role> = guild.roles.values().collect();
    upsert_roles(&mut transaction, &roles).await?;
    let role_ids: Vec<i64> = roles.iter().map(|role| role.id.to_i64()).collect();
    prune_stale_roles(&mut transaction, guild_id, &role_ids).await?;

    let channels: Vec<&GuildChannel> = guild.channels.values().collect();
    upsert_channels(&mut transaction, &channels).await?;
    let channel_ids: Vec<i64> = channels.iter().map(|channel| channel.id.to_i64()).collect();
    prune_stale_channels(&mut transaction, guild_id, &channel_ids).await?;
    sync_channel_permissions(&mut transaction, guild_id, &channels, None).await?;

    sync_guild_user_roles(&mut transaction, guild).await?;

    transaction.commit().await?;
    Ok(())
}

/// Self-healing full re-sync of the guild cache tables from the live serenity
/// cache. Discord replays nothing after a gateway gap, and a long-lived bot
/// would otherwise keep a stale snapshot of roles, channels, permission
/// overwrites and member role assignments until its next restart.
pub(crate) async fn resync_guild_cache(custom: &crate::Custom) -> DbResult<()> {
    let guild_ids: Vec<GuildId> = custom.cache.guilds();
    let mut guild_cached = Vec::with_capacity(guild_ids.len());
    for guild_id in &guild_ids {
        if let Some(guild) = guild_id.to_guild_cached(&custom.cache) {
            guild_cached.push(guild.to_owned());
        }
    }

    sync_info(&custom.pool, &guild_cached).await?;
    sync_present_guild_ids(&custom.pool, &guild_ids).await?;
    Ok(())
}

/// The gateway periodically re-sends `GuildCreate` for guilds it already
/// knows; a guild that becomes available again mid-run lands here too, so this
/// is also where a full cache sync happens for one guild.
pub(crate) async fn sync_new_guild(pool: &Pool<Postgres>, guild: &Guild) -> DbResult<()> {
    sync_info(pool, std::slice::from_ref(guild)).await?;
    upsert_guilds_present(pool, &[guild.id.to_i64()]).await
}

/// The bot leaving a guild (`guild_delete` without guild data). Only the
/// presence row is removed: recordings and their channel/role rows are the
/// product, and their foreign keys are not worth cascading away.
pub(crate) async fn remove_guild_present(
    pool: &Pool<Postgres>,
    guild_id: serenity::model::id::GuildId,
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM guilds_present WHERE guild_id = $1",
        guild_id.to_i64()
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn sync_live_role(pool: &Pool<Postgres>, role: &Role) -> DbResult<()> {
    let mut connection = pool.acquire().await?;
    upsert_roles(&mut connection, &[role]).await
}

pub(crate) async fn delete_live_role(pool: &Pool<Postgres>, role_id: RoleId) -> DbResult<()> {
    let mut transaction = pool.begin().await?;
    let role_id = role_id.to_i64();

    // channel_permissions.target_id is intentionally polymorphic and has no
    // foreign key to roles, so remove these rows explicitly.
    sqlx::query!(
        "DELETE FROM channel_permissions WHERE kind = 'role' AND target_id = $1",
        role_id
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!("DELETE FROM roles WHERE role_id = $1", role_id)
        .execute(&mut *transaction)
        .await?;

    transaction.commit().await?;
    Ok(())
}

/// Member role assignments for a full sync. Discord sends a large guild's
/// members only partially, so assignments are pruned for every member only
/// when the cache holds the whole member list; otherwise only cached members'
/// assignments are corrected and absent members keep theirs.
async fn sync_guild_user_roles(connection: &mut PgConnection, guild: &Guild) -> DbResult<()> {
    let mut user_ids = Vec::new();
    let mut role_ids = Vec::new();
    for (user_id, member) in &guild.members {
        for role_id in &member.roles {
            user_ids.push(user_id.to_i64());
            role_ids.push(role_id.to_i64());
        }
    }
    let cached_members: Vec<i64> = guild
        .members
        .keys()
        .map(|user_id| user_id.to_i64())
        .collect();
    let complete = u64::try_from(cached_members.len()).unwrap_or(u64::MAX) >= guild.member_count;

    write_user_roles(
        connection,
        guild.id.to_i64(),
        &user_ids,
        &role_ids,
        (!complete).then_some(cached_members.as_slice()),
    )
    .await
}

/// Make `user_roles` match the given (user, role) pairs for one guild,
/// writing only the difference. `only_users` limits removals to those
/// members; `None` removes every other assignment in the guild.
async fn write_user_roles(
    connection: &mut PgConnection,
    guild_id: i64,
    user_ids: &[i64],
    role_ids: &[i64],
    only_users: Option<&[i64]>,
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM user_roles ur
          USING roles r
         WHERE ur.role_id = r.role_id
           AND r.guild_id = $1
           AND ($4::bigint[] IS NULL OR ur.user_id = ANY($4))
           AND NOT EXISTS (
               SELECT 1
                 FROM UNNEST($2::bigint[], $3::bigint[]) AS wanted(user_id, role_id)
                WHERE wanted.user_id = ur.user_id
                  AND wanted.role_id = ur.role_id
           )",
        guild_id,
        user_ids,
        role_ids,
        only_users
    )
    .execute(&mut *connection)
    .await?;

    // Joining through roles keeps an out-of-order unknown role fail-closed:
    // known revoked roles stay removed without violating the foreign key.
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT wanted.user_id, r.role_id
           FROM UNNEST($2::bigint[], $3::bigint[]) AS wanted(user_id, role_id)
           JOIN roles r ON r.role_id = wanted.role_id AND r.guild_id = $1
         ON CONFLICT (user_id, role_id) DO NOTHING",
        guild_id,
        user_ids,
        role_ids
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(crate) async fn sync_live_member_roles(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    user_id: UserId,
    role_ids: &[RoleId],
) -> DbResult<()> {
    let user_id = user_id.to_i64();
    let role_ids: Vec<i64> = role_ids.iter().copied().map(ToI64::to_i64).collect();
    let user_ids = vec![user_id; role_ids.len()];

    let mut transaction = pool.begin().await?;
    write_user_roles(
        &mut transaction,
        guild_id.to_i64(),
        &user_ids,
        &role_ids,
        Some(&[user_id]),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn delete_live_member(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    user_id: UserId,
) -> DbResult<()> {
    let mut transaction = pool.begin().await?;
    let guild_id = guild_id.to_i64();
    let user_id = user_id.to_i64();

    sqlx::query!(
        "DELETE FROM user_roles ur
          USING roles r
         WHERE ur.role_id = r.role_id
           AND ur.user_id = $1
           AND r.guild_id = $2",
        user_id,
        guild_id
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM user_guilds WHERE id = $1 AND user_id = $2",
        guild_id,
        user_id
    )
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn sync_guild_owner(
    pool: &Pool<Postgres>,
    guild_id: GuildId,
    owner_id: UserId,
) -> DbResult<()> {
    let mut transaction = pool.begin().await?;
    upsert_guild_owner(&mut transaction, guild_id.to_i64(), owner_id.to_i64()).await?;
    transaction.commit().await?;
    Ok(())
}

/// Record a guild's owner. `web-server` grants owner rights from either
/// `guilds.owner_id` or the OAuth-snapshot `user_guilds.owner` flag, so both
/// move together; otherwise an ownership change the gateway never delivered
/// (one made while the bot was offline) would leave the previous owner with
/// full permissions until their next login. Only rows that disagree are
/// written.
async fn upsert_guild_owner(
    connection: &mut PgConnection,
    guild_id: i64,
    owner_id: i64,
) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO guilds (id, owner_id)
         VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE SET owner_id = EXCLUDED.owner_id
          WHERE guilds.owner_id IS DISTINCT FROM EXCLUDED.owner_id",
        guild_id,
        owner_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query!(
        "UPDATE user_guilds
            SET owner = (user_id = $2)
          WHERE id = $1
            AND owner IS DISTINCT FROM (user_id = $2)",
        guild_id,
        owner_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn upsert_roles(connection: &mut PgConnection, roles: &[&Role]) -> DbResult<()> {
    let mut guild_ids = Vec::with_capacity(roles.len());
    let mut role_ids = Vec::with_capacity(roles.len());
    let mut permissions = Vec::with_capacity(roles.len());
    let mut names = Vec::with_capacity(roles.len());
    let mut colors = Vec::with_capacity(roles.len());
    let mut secondary_colors = Vec::with_capacity(roles.len());
    let mut tertiary_colors = Vec::with_capacity(roles.len());
    for role in roles {
        guild_ids.push(role.guild_id.to_i64());
        role_ids.push(role.id.to_i64());
        permissions.push(role.permissions.bits().to_i64());
        names.push(role.name.clone());
        colors.push(i64::from(role.colours.primary_colour.0));
        secondary_colors.push(role.colours.secondary_colour.map(|c| i64::from(c.0)));
        tertiary_colors.push(role.colours.tertiary_colour.map(|c| i64::from(c.0)));
    }

    sqlx::query!(
        "INSERT INTO roles
            (guild_id, role_id, permission, name, color, color_secondary, color_tertiary)
         SELECT *
           FROM UNNEST($1::bigint[], $2::bigint[], $3::bigint[], $4::text[],
                       $5::bigint[], $6::bigint[], $7::bigint[])
         ON CONFLICT (role_id) DO UPDATE SET
             guild_id = EXCLUDED.guild_id,
             permission = EXCLUDED.permission,
             name = EXCLUDED.name,
             color = EXCLUDED.color,
             color_secondary = EXCLUDED.color_secondary,
             color_tertiary = EXCLUDED.color_tertiary
          WHERE (roles.guild_id, roles.permission, roles.name, roles.color,
                 roles.color_secondary, roles.color_tertiary)
                IS DISTINCT FROM
                (EXCLUDED.guild_id, EXCLUDED.permission, EXCLUDED.name, EXCLUDED.color,
                 EXCLUDED.color_secondary, EXCLUDED.color_tertiary)",
        &guild_ids,
        &role_ids,
        &permissions,
        &names,
        &colors,
        &secondary_colors as &[Option<i64>],
        &tertiary_colors as &[Option<i64>]
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn upsert_channels(
    connection: &mut PgConnection,
    channels: &[&GuildChannel],
) -> DbResult<()> {
    let channel_ids: Vec<i64> = channels.iter().map(|channel| channel.id.to_i64()).collect();
    let guild_ids: Vec<i64> = channels
        .iter()
        .map(|channel| channel.guild_id.to_i64())
        .collect();
    let kinds: Vec<i32> = channels
        .iter()
        .map(|channel| i32::from(u8::from(channel.kind)))
        .collect();
    let names: Vec<String> = channels
        .iter()
        .map(|channel| channel.name().to_string())
        .collect();

    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::int[], $4::text[])
         ON CONFLICT (channel_id) DO UPDATE SET
             guild_id = EXCLUDED.guild_id,
             type = EXCLUDED.type,
             name = EXCLUDED.name
          WHERE (channels.guild_id, channels.type, channels.name)
                IS DISTINCT FROM (EXCLUDED.guild_id, EXCLUDED.type, EXCLUDED.name)",
        &channel_ids,
        &guild_ids,
        &kinds,
        &names
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Make `channel_permissions` match the given channels' overwrites, writing
/// only the difference. `only_channel` limits removals to that channel;
/// `None` removes stale overwrites from every channel in the guild.
async fn sync_channel_permissions(
    connection: &mut PgConnection,
    guild_id: i64,
    channels: &[&GuildChannel],
    only_channel: Option<i64>,
) -> DbResult<()> {
    let mut channel_ids = Vec::new();
    let mut target_ids = Vec::new();
    let mut kinds = Vec::new();
    let mut allows = Vec::new();
    let mut denies = Vec::new();
    for channel in channels {
        for overwrite in &channel.permission_overwrites {
            let (kind, target_id) = match overwrite.kind {
                PermissionOverwriteType::Member(target_id) => ("user", target_id.to_i64()),
                PermissionOverwriteType::Role(target_id) => ("role", target_id.to_i64()),
                _ => {
                    error!(
                        channel_id = channel.id.get(),
                        "unknown permission overwrite type"
                    );
                    continue;
                }
            };
            channel_ids.push(channel.id.to_i64());
            target_ids.push(target_id);
            kinds.push(kind.to_string());
            allows.push(overwrite.allow.bits().to_i64());
            denies.push(overwrite.deny.bits().to_i64());
        }
    }

    sqlx::query!(
        "DELETE FROM channel_permissions cp
          USING channels c
         WHERE cp.channel_id = c.channel_id
           AND c.guild_id = $1
           AND ($4::bigint IS NULL OR cp.channel_id = $4)
           AND NOT EXISTS (
               SELECT 1
                 FROM UNNEST($2::bigint[], $3::bigint[]) AS wanted(channel_id, target_id)
                WHERE wanted.channel_id = cp.channel_id
                  AND wanted.target_id = cp.target_id
           )",
        guild_id,
        &channel_ids,
        &target_ids,
        only_channel
    )
    .execute(&mut *connection)
    .await?;

    sqlx::query!(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::text[], $4::bigint[], $5::bigint[])
         ON CONFLICT (channel_id, target_id) DO UPDATE SET
             kind = EXCLUDED.kind,
             allow = EXCLUDED.allow,
             deny = EXCLUDED.deny
          WHERE (channel_permissions.kind, channel_permissions.allow, channel_permissions.deny)
                IS DISTINCT FROM (EXCLUDED.kind, EXCLUDED.allow, EXCLUDED.deny)",
        &channel_ids,
        &target_ids,
        &kinds,
        &allows,
        &denies
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(crate) async fn sync_live_channel(
    pool: &Pool<Postgres>,
    channel: &GuildChannel,
) -> DbResult<()> {
    let mut transaction = pool.begin().await?;
    upsert_channels(&mut transaction, &[channel]).await?;
    sync_channel_permissions(
        &mut transaction,
        channel.guild_id.to_i64(),
        &[channel],
        Some(channel.id.to_i64()),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn delete_live_channel(
    pool: &Pool<Postgres>,
    channel_id: serenity::model::id::ChannelId,
) -> DbResult<()> {
    // The channel_permissions foreign key cascades this deletion.
    sqlx::query!(
        "DELETE FROM channels WHERE channel_id = $1",
        channel_id.to_i64()
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove the guild's roles missing from `role_ids`; member assignments
/// cascade. An empty list removes them all (`= ANY('{}')` is false).
async fn prune_stale_roles(
    connection: &mut PgConnection,
    guild_id: i64,
    role_ids: &[i64],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM roles WHERE guild_id = $1 AND NOT (role_id = ANY($2))",
        guild_id,
        role_ids
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Remove the guild's channels missing from `channel_ids`; overwrites
/// cascade. An empty list removes them all.
async fn prune_stale_channels(
    connection: &mut PgConnection,
    guild_id: i64,
    channel_ids: &[i64],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM channels WHERE guild_id = $1 AND NOT (channel_id = ANY($2))",
        guild_id,
        channel_ids
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn prune_stale_roles_for_test(
    pool: &Pool<Postgres>,
    guild_id: i64,
    role_ids: &[i64],
) -> DbResult<()> {
    let mut connection = pool.acquire().await?;
    prune_stale_roles(&mut connection, guild_id, role_ids).await
}

#[cfg(test)]
pub(crate) async fn prune_stale_channels_for_test(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_ids: &[i64],
) -> DbResult<()> {
    let mut connection = pool.acquire().await?;
    prune_stale_channels(&mut connection, guild_id, channel_ids).await
}

pub async fn update_guild_present(guilds: Vec<UnavailableGuild>, pool: &Pool<Postgres>) {
    let guild_ids: Vec<GuildId> = guilds.into_iter().map(|guild| guild.id).collect();
    if let Err(err) = sync_present_guild_ids(pool, &guild_ids).await {
        error!(error = %err, "failed to update present guilds");
    }
}

/// `guilds_present` is exactly the bot's guild set: upsert the given ids and
/// prune everything else.
pub(crate) async fn sync_present_guild_ids(
    pool: &Pool<Postgres>,
    guild_ids: &[GuildId],
) -> DbResult<()> {
    let guild_ids: Vec<i64> = guild_ids.iter().copied().map(ToI64::to_i64).collect();
    upsert_guilds_present(pool, &guild_ids).await?;
    if guild_ids.is_empty() {
        sqlx::query!("DELETE FROM guilds_present")
            .execute(pool)
            .await?;
    } else {
        sqlx::query!(
            "DELETE FROM guilds_present WHERE NOT (guild_id = ANY($1))",
            &guild_ids
        )
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn upsert_guilds_present(pool: &Pool<Postgres>, guild_ids: &[i64]) -> DbResult<()> {
    sqlx::query!(
        "INSERT INTO guilds_present (guild_id)
         SELECT * FROM UNNEST($1::bigint[])
         ON CONFLICT (guild_id) DO NOTHING",
        guild_ids
    )
    .execute(pool)
    .await?;
    Ok(())
}

const DEFAULT_GUILD_CACHE_RESYNC_SECONDS: u64 = 900;

/// Periodically re-sync the guild cache tables from the live serenity cache so
/// a long-running bot self-heals events it missed (gateway gaps, downtime,
/// events Discord does not replay). Interval configurable via
/// `SAKIOT_GUILD_CACHE_RESYNC_SECONDS`; `0` disables the task.
pub(crate) fn spawn_cache_resync(
    custom: crate::Custom,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let interval_seconds = std::env::var("SAKIOT_GUILD_CACHE_RESYNC_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_GUILD_CACHE_RESYNC_SECONDS);

    tokio::spawn(async move {
        if interval_seconds == 0 {
            return;
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(err) = resync_guild_cache(&custom).await {
                        error!(error = %err, "periodic guild cache resync failed");
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }
    })
}

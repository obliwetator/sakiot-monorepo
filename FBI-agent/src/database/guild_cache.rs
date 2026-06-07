use crate::cast::ToI64;
use crate::database::DbResult;
use crate::event_handler::Handler;
use serenity::{
    all::UnavailableGuild,
    model::prelude::{Guild, GuildId},
    prelude::Context,
};
use sqlx::{Pool, Postgres};
use tracing::error;

const BIND_LIMIT: usize = 65535;

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

    if let Err(err) = sync_info(handler, &guild_cached).await {
        error!(error = %err, "failed to sync guild cache");
    }
}

async fn sync_info(handler: &Handler, guild_cached: &[Guild]) -> DbResult<()> {
    update_guilds(guild_cached, handler).await?;
    update_roles(guild_cached, handler).await?;
    update_user_roles(guild_cached, handler).await?;
    update_channels(guild_cached, handler).await?;
    update_permissions(guild_cached, handler).await?;
    Ok(())
}

async fn update_roles(guild_cached: &[Guild], handler: &Handler) -> DbResult<()> {
    for guild in guild_cached {
        let roles: Vec<_> = guild.roles.iter().collect();
        for chunk in roles.chunks(BIND_LIMIT / 4) {
            let mut query_builder: sqlx::QueryBuilder<Postgres> =
                sqlx::QueryBuilder::new("INSERT INTO roles (guild_id, role_id, permission, name) ");
            query_builder
                .push_values(chunk, |mut b, role| {
                    b.push_bind(role.1.guild_id.to_i64())
                        .push_bind(role.0.to_i64())
                        .push_bind(role.1.permissions.bits().to_i64())
                        .push_bind(&role.1.name);
                })
                .push(
                    " ON CONFLICT (role_id) DO UPDATE SET \
                     guild_id = EXCLUDED.guild_id, \
                     permission = EXCLUDED.permission, \
                     name = EXCLUDED.name",
                );

            query_builder.build().execute(&handler.database).await?;
        }

        let role_ids: Vec<i64> = roles.iter().map(|role| role.0.to_i64()).collect();
        prune_stale_roles(&handler.database, guild.id.to_i64(), &role_ids).await?;
    }

    Ok(())
}

async fn update_user_roles(guild_cached: &[Guild], handler: &Handler) -> DbResult<()> {
    for guild in guild_cached {
        delete_user_roles_for_guild(&handler.database, guild.id.to_i64()).await?;

        let mut user_roles = Vec::new();
        for (user_id, user) in guild.members.iter() {
            for role in &user.roles {
                user_roles.push((user_id.to_i64(), role.to_i64()));
            }
        }

        for chunk in user_roles.chunks(BIND_LIMIT / 2) {
            let mut query_builder: sqlx::QueryBuilder<Postgres> =
                sqlx::QueryBuilder::new("INSERT INTO user_roles (user_id, role_id) ");

            query_builder
                .push_values(chunk, |mut b, pair| {
                    b.push_bind(pair.0).push_bind(pair.1);
                })
                .push(" ON CONFLICT (user_id, role_id) DO NOTHING");

            query_builder.build().execute(&handler.database).await?;
        }
    }

    Ok(())
}

async fn update_permissions(guild_cached: &[Guild], handler: &Handler) -> DbResult<()> {
    for guild in guild_cached {
        delete_channel_permissions_for_guild(&handler.database, guild.id.to_i64()).await?;

        let mut overwrites = Vec::new();
        for channel in guild.channels.values() {
            for p in &channel.permission_overwrites {
                let kind = match p.kind {
                    serenity::model::prelude::PermissionOverwriteType::Member(target_id) => {
                        ("user", target_id.to_i64())
                    }
                    serenity::model::prelude::PermissionOverwriteType::Role(target_id) => {
                        ("role", target_id.to_i64())
                    }
                    _ => {
                        error!(
                            channel_id = channel.id.get(),
                            "unknown permission overwrite type"
                        );
                        continue;
                    }
                };
                overwrites.push((
                    channel.id.to_i64(),
                    kind.1,
                    kind.0,
                    p.allow.bits().to_i64(),
                    p.deny.bits().to_i64(),
                ));
            }
        }

        for chunk in overwrites.chunks(BIND_LIMIT / 5) {
            let mut query_builder: sqlx::QueryBuilder<Postgres> = sqlx::QueryBuilder::new(
                "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny) ",
            );

            query_builder
                .push_values(chunk, |mut b, overwrite| {
                    b.push_bind(overwrite.0)
                        .push_bind(overwrite.1)
                        .push_bind(overwrite.2)
                        .push_bind(overwrite.3)
                        .push_bind(overwrite.4);
                })
                .push(" ON CONFLICT (channel_id, target_id) DO UPDATE SET kind = EXCLUDED.kind, allow = EXCLUDED.allow, deny = EXCLUDED.deny");

            query_builder.build().execute(&handler.database).await?;
        }
    }

    Ok(())
}

async fn update_guilds(guild_cached: &[Guild], handler: &Handler) -> DbResult<()> {
    if guild_cached.is_empty() {
        return Ok(());
    }

    let mut query_builder: sqlx::QueryBuilder<Postgres> =
        sqlx::QueryBuilder::new("INSERT INTO guilds (id, owner_id) ");

    query_builder
        .push_values(guild_cached.iter().take(BIND_LIMIT / 2), |mut b, guild| {
            b.push_bind(guild.id.to_i64())
                .push_bind(guild.owner_id.to_i64());
        })
        .push(" ON CONFLICT (id) DO UPDATE SET owner_id = EXCLUDED.owner_id");

    query_builder.build().execute(&handler.database).await?;

    Ok(())
}

async fn update_channels(guild_cached: &[Guild], handler: &Handler) -> DbResult<()> {
    for guild in guild_cached {
        let channels: Vec<_> = guild.channels.values().collect();
        for chunk in channels.chunks(BIND_LIMIT / 4) {
            let mut query_builder: sqlx::QueryBuilder<Postgres> =
                sqlx::QueryBuilder::new("INSERT INTO channels (channel_id, guild_id, type, name) ");

            query_builder
                .push_values(chunk, |mut b, channel| {
                    b.push_bind(channel.id.to_i64())
                        .push_bind(channel.guild_id.to_i64())
                        .push_bind(u8::from(channel.kind) as i32)
                        .push_bind(channel.name());
                })
                .push(
                    " ON CONFLICT (channel_id) DO UPDATE SET \
                     guild_id = EXCLUDED.guild_id, \
                     type = EXCLUDED.type, \
                     name = EXCLUDED.name",
                );

            query_builder.build().execute(&handler.database).await?;
        }

        let channel_ids: Vec<i64> = channels
            .iter()
            .map(|channel| channel.id.to_i64())
            .collect();
        prune_stale_channels(&handler.database, guild.id.to_i64(), &channel_ids).await?;
    }

    Ok(())
}

async fn prune_stale_roles(pool: &Pool<Postgres>, guild_id: i64, role_ids: &[i64]) -> DbResult<()> {
    if role_ids.is_empty() {
        sqlx::query("DELETE FROM roles WHERE guild_id = $1")
            .bind(guild_id)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM roles WHERE guild_id = $1 AND NOT (role_id = ANY($2))")
            .bind(guild_id)
            .bind(role_ids)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn delete_user_roles_for_guild(pool: &Pool<Postgres>, guild_id: i64) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM user_roles ur
          USING roles r
         WHERE ur.role_id = r.role_id
           AND r.guild_id = $1",
    )
    .bind(guild_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn delete_channel_permissions_for_guild(
    pool: &Pool<Postgres>,
    guild_id: i64,
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM channel_permissions cp
          USING channels c
         WHERE cp.channel_id = c.channel_id
           AND c.guild_id = $1",
    )
    .bind(guild_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn prune_stale_channels(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_ids: &[i64],
) -> DbResult<()> {
    if channel_ids.is_empty() {
        sqlx::query("DELETE FROM channels WHERE guild_id = $1")
            .bind(guild_id)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM channels WHERE guild_id = $1 AND NOT (channel_id = ANY($2))")
            .bind(guild_id)
            .bind(channel_ids)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn prune_stale_roles_for_test(
    pool: &Pool<Postgres>,
    guild_id: i64,
    role_ids: &[i64],
) -> DbResult<()> {
    prune_stale_roles(pool, guild_id, role_ids).await
}

#[cfg(test)]
pub(crate) async fn prune_stale_channels_for_test(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channel_ids: &[i64],
) -> DbResult<()> {
    prune_stale_channels(pool, guild_id, channel_ids).await
}

pub async fn update_guild_present(guilds: Vec<UnavailableGuild>, handler: &Handler) {
    if let Err(err) = sync_guild_present(guilds, handler).await {
        error!(error = %err, "failed to update present guilds");
    }
}

async fn sync_guild_present(guilds: Vec<UnavailableGuild>, handler: &Handler) -> DbResult<()> {
    let guild_ids: Vec<i64> = guilds
        .into_iter()
        .map(|guild| guild.id.to_i64())
        .collect();

    for chunk in guild_ids.chunks(BIND_LIMIT) {
        let mut query_builder: sqlx::QueryBuilder<Postgres> =
            sqlx::QueryBuilder::new("INSERT INTO guilds_present (guild_id) ");

        query_builder
            .push_values(chunk, |mut b, guild_id| {
                b.push_bind(*guild_id);
            })
            .push(" ON CONFLICT (guild_id) DO NOTHING");

        query_builder.build().execute(&handler.database).await?;
    }

    if guild_ids.is_empty() {
        sqlx::query("DELETE FROM guilds_present")
            .execute(&handler.database)
            .await?;
    } else {
        sqlx::query("DELETE FROM guilds_present WHERE NOT (guild_id = ANY($1))")
            .bind(&guild_ids)
            .execute(&handler.database)
            .await?;
    }

    Ok(())
}

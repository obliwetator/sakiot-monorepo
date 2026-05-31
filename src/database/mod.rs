pub mod user_names;

use crate::event_handler::Handler;
use serenity::{
    all::UnavailableGuild,
    model::prelude::{Guild, GuildId},
    prelude::Context,
};
use sqlx::Postgres;
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

    update_guilds(&guild_cached, handler).await;
    update_roles(&guild_cached, handler).await;

    // TODO: Remove roles that are not present
    // TODO: Remove roles that are not present while the bot is running
    update_user_roles(&guild_cached, handler).await;
    update_channels(&guild_cached, handler).await;
    update_permissions(&guild_cached, handler).await;
}

async fn update_roles(guild_cached: &[Guild], handler: &Handler) {
    for guild in guild_cached {
        let mut query_builder: sqlx::QueryBuilder<Postgres> =
            sqlx::QueryBuilder::new("INSERT INTO roles (guild_id, role_id, permission, name) ");
        query_builder
            .push_values(guild.roles.iter().take(BIND_LIMIT / 4), |mut b, role| {
                b.push_bind(role.1.guild_id.get() as i64)
                    .push_bind(role.0.get() as i64)
                    .push_bind(role.1.permissions.bits() as i64)
                    .push_bind(&role.1.name);
            })
            .push(" ON CONFLICT (role_id) DO UPDATE SET permission=EXCLUDED.permission");

        let query = query_builder.build();

        if let Err(err) = query.execute(&handler.database).await {
            error!(guild_id = guild.id.get(), error = %err, "failed to update roles");
        }
    }
}

async fn update_user_roles(guild_cached: &[Guild], handler: &Handler) {
    for guild in guild_cached {
        let mut user_roles = Vec::new();
        for (user_id, user) in guild.members.iter() {
            for role in &user.roles {
                user_roles.push((user_id.get() as i64, role.get() as i64));
            }
        }

        if !user_roles.is_empty() {
            for chunk in user_roles.chunks(BIND_LIMIT / 2) {
                let mut query_builder: sqlx::QueryBuilder<Postgres> =
                    sqlx::QueryBuilder::new("INSERT INTO user_roles (user_id, role_id) ");

                query_builder
                    .push_values(chunk, |mut b, pair| {
                        b.push_bind(pair.0).push_bind(pair.1);
                    })
                    .push(" ON CONFLICT (user_id, role_id) DO UPDATE SET role_id=EXCLUDED.role_id");

                let query = query_builder.build();
                if let Err(err) = query.execute(&handler.database).await {
                    error!(guild_id = guild.id.get(), error = %err, "failed to update user roles");
                }
            }
        }
    }
}

async fn update_permissions(guild_cached: &[Guild], handler: &Handler) {
    for guild in guild_cached {
        let mut overwrites = Vec::new();
        for channel in guild.channels.values() {
            for p in &channel.permission_overwrites {
                let kind = match p.kind {
                    serenity::model::prelude::PermissionOverwriteType::Member(target_id) => {
                        ("user", target_id.get() as i64)
                    }
                    serenity::model::prelude::PermissionOverwriteType::Role(target_id) => {
                        ("role", target_id.get() as i64)
                    }
                    _ => {
                        error!(
                            channel_id = channel.id.get(),
                            "unknown permission overwrite type"
                        );
                        ("unknown", 0)
                    }
                };
                overwrites.push((
                    channel.id.get() as i64,
                    kind.1,
                    kind.0,
                    p.allow.bits() as i64,
                    p.deny.bits() as i64,
                ));
            }
        }

        if !overwrites.is_empty() {
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
                    .push(" ON CONFLICT (channel_id, target_id) DO UPDATE SET allow = EXCLUDED.allow, deny = EXCLUDED.deny");

                let query = query_builder.build();

                if let Err(err) = query.execute(&handler.database).await {
                    error!(guild_id = guild.id.get(), error = %err, "failed to update channel permissions");
                }
            }
        }
    }
}

async fn update_guilds(guild_cached: &[Guild], handler: &Handler) {
    let mut query_builder: sqlx::QueryBuilder<Postgres> =
        sqlx::QueryBuilder::new("INSERT INTO guilds (id, owner_id) ");

    query_builder
        .push_values(guild_cached.iter().take(BIND_LIMIT / 2), |mut b, guild| {
            b.push_bind(guild.id.get() as i64)
                .push_bind(guild.owner_id.get() as i64);
        })
        .push(" ON CONFLICT DO NOTHING ");

    let query = query_builder.build();

    if let Err(err) = query.execute(&handler.database).await {
        error!(error = %err, "failed to update guilds");
    }
}

async fn update_channels(guild_cached: &[Guild], handler: &Handler) {
    // "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny) ",

    for guild in guild_cached {
        let mut query_builder: sqlx::QueryBuilder<Postgres> =
            sqlx::QueryBuilder::new("INSERT INTO channels (channel_id, guild_id, type, name) ");
        let ch = &guild.channels;

        query_builder
            .push_values(ch.iter().take(BIND_LIMIT / 4), |mut b, channel| {
                b.push_bind(channel.1.id.get() as i64)
                    .push_bind(channel.1.guild_id.get() as i64)
                    .push_bind(u8::from(channel.1.kind) as i32)
                    .push_bind(channel.1.name());
            })
            .push(" ON CONFLICT (channel_id) DO UPDATE SET name=EXCLUDED.name ");

        let query = query_builder.build();

        if let Err(err) = query.execute(&handler.database).await {
            error!(guild_id = guild.id.get(), error = %err, "failed to update channels");
        }
    }
}

pub async fn update_guild_present(guilds: Vec<UnavailableGuild>, handler: &Handler) {
    // TODO. We only check the guild we are currently in. Check if the bot has left/kicked from any guild.
    let mut query_builder: sqlx::QueryBuilder<Postgres> =
        sqlx::QueryBuilder::new("INSERT INTO guilds_present (guild_id) ");

    query_builder
        .push_values(guilds.into_iter().take(BIND_LIMIT), |mut b, guild| {
            b.push_bind(guild.id.get() as i64);
        })
        .push(" ON CONFLICT (guild_id) DO NOTHING");

    let query = query_builder.build();

    if let Err(err) = query.execute(&handler.database).await {
        error!(error = %err, "failed to update present guilds");
    }
}

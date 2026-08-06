use serenity::client::Context;
use tracing::error;

use crate::event_handler::Handler;

pub async fn guild_ban_addition(
    _self: &Handler,
    _ctx: Context,
    _guild_id: serenity::model::id::GuildId,
    _banned_user: serenity::model::prelude::User,
) {
}

pub async fn guild_ban_removal(
    _self: &Handler,
    _ctx: Context,
    _guild_id: serenity::model::id::GuildId,
    _unbanned_user: serenity::model::prelude::User,
) {
}

pub async fn guild_create(
    handler: &Handler,
    _ctx: Context,
    guild: serenity::model::guild::Guild,
    _is_new: Option<bool>,
) {
    if let Err(err) = crate::database::guild_cache::sync_new_guild(&handler.database, &guild).await
    {
        error!(
            error = %err,
            guild_id = guild.id.get(),
            "failed to sync guild cache on guild_create"
        );
    }
}

pub async fn guild_delete(
    handler: &Handler,
    _ctx: Context,
    incomplete: serenity::model::guild::UnavailableGuild,
    full: Option<serenity::model::guild::Guild>,
) {
    // full = Some means the guild is temporarily unavailable and will come
    // back as a guild_create; only an actual removal (None) drops it from
    // guilds_present.
    if full.is_some() {
        return;
    }
    if let Err(err) =
        crate::database::guild_cache::remove_guild_present(&handler.database, incomplete.id).await
    {
        error!(
            error = %err,
            guild_id = incomplete.id.get(),
            "failed to remove departed guild from guilds_present"
        );
    }
}

pub async fn guild_member_removal(
    handler: &Handler,
    _ctx: Context,
    guild_id: serenity::model::id::GuildId,
    user: serenity::model::prelude::User,
    _member_data_if_available: Option<serenity::model::guild::Member>,
) {
    if let Err(err) =
        crate::database::guild_cache::delete_live_member(&handler.database, guild_id, user.id).await
    {
        error!(
            error = %err,
            guild_id = guild_id.get(),
            user_id = user.id.get(),
            "failed to remove guild member from cache"
        );
    }
}

pub async fn guild_member_addition(
    handler: &Handler,
    _ctx: Context,
    new_member: serenity::model::guild::Member,
) {
    if let Err(err) = crate::database::guild_cache::sync_live_member_roles(
        &handler.database,
        new_member.guild_id,
        new_member.user.id,
        &new_member.roles,
    )
    .await
    {
        error!(
            error = %err,
            guild_id = new_member.guild_id.get(),
            user_id = new_member.user.id.get(),
            "failed to sync added member roles"
        );
    }

    if new_member.user.bot {
        return;
    }
    crate::database::user_names::observe(
        &handler.database,
        new_member.guild_id.get(),
        &new_member.user,
        Some(&new_member),
    )
    .await;
}

pub async fn guild_member_update(
    handler: &Handler,
    _ctx: Context,
    new: Option<serenity::model::guild::Member>,
    event: serenity::model::event::GuildMemberUpdateEvent,
) {
    if let Err(err) = crate::database::guild_cache::sync_live_member_roles(
        &handler.database,
        event.guild_id,
        event.user.id,
        &event.roles,
    )
    .await
    {
        error!(
            error = %err,
            guild_id = event.guild_id.get(),
            user_id = event.user.id.get(),
            "failed to sync updated member roles"
        );
    }

    if event.user.bot {
        return;
    }
    crate::database::user_names::observe(
        &handler.database,
        event.guild_id.get(),
        &event.user,
        new.as_ref(),
    )
    .await;
}

pub async fn guild_members_chunk(
    _self: &Handler,
    _ctx: Context,
    chunk: serenity::model::event::GuildMembersChunkEvent,
) {
    for member in chunk.members.into_values() {
        if member.user.bot {
            continue;
        }
        crate::database::user_names::observe(
            &_self.database,
            chunk.guild_id.get(),
            &member.user,
            Some(&member),
        )
        .await;
    }
}

pub async fn guild_update(
    handler: &Handler,
    _ctx: Context,
    _old_data_if_available: Option<serenity::model::guild::Guild>,
    new_guild: serenity::model::guild::PartialGuild,
) {
    if let Err(err) = crate::database::guild_cache::sync_guild_owner(
        &handler.database,
        new_guild.id,
        new_guild.owner_id,
    )
    .await
    {
        error!(
            guild_id = new_guild.id.get(),
            owner_id = new_guild.owner_id.get(),
            "failed to update guild owner: {}",
            err
        );
    }
}

use serenity::client::Context;
use tracing::error;

use crate::event_handler::Handler;

pub async fn guild_create(
    handler: &Handler,
    ctx: Context,
    guild: serenity::model::guild::Guild,
    _is_new: Option<bool>,
) {
    if handler.runtime.maintains_caches()
        && let Err(err) =
            crate::database::guild_cache::sync_new_guild(&handler.database, &guild).await
    {
        error!(
            error = %err,
            guild_id = guild.id.get(),
            "failed to sync guild cache on guild_create"
        );
    }
    handler
        .projections
        .guild_available(&ctx.cache, guild.id, ctx.shard.clone())
        .await;
}

pub async fn guild_delete(
    handler: &Handler,
    _ctx: Context,
    incomplete: serenity::model::guild::UnavailableGuild,
    _full: Option<serenity::model::guild::Guild>,
) {
    // Discord sends GUILD_DELETE with unavailable=true when a guild is
    // temporarily unavailable during an outage; the guild comes back as a
    // guild_create and must stay in guilds_present. Only an actual removal
    // (unavailable=false, typically with the full cached guild attached)
    // drops it from guilds_present.
    if incomplete.unavailable {
        return;
    }
    if handler.runtime.maintains_caches()
        && let Err(err) =
            crate::database::guild_cache::remove_guild_present(&handler.database, incomplete.id)
                .await
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
    ctx: Context,
    guild_id: serenity::model::id::GuildId,
    user: serenity::model::prelude::User,
    _member_data_if_available: Option<serenity::model::guild::Member>,
) {
    if handler.runtime.maintains_caches()
        && let Err(err) =
            crate::database::guild_cache::delete_live_member(&handler.database, guild_id, user.id)
                .await
    {
        error!(
            error = %err,
            guild_id = guild_id.get(),
            user_id = user.id.get(),
            "failed to remove guild member from cache"
        );
    }
    handler
        .projections
        .member_left(&ctx.cache, guild_id, user.id)
        .await;
}

pub async fn guild_member_addition(
    handler: &Handler,
    ctx: Context,
    new_member: serenity::model::guild::Member,
) {
    if handler.runtime.maintains_caches()
        && let Err(err) = crate::database::guild_cache::sync_live_member_roles(
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
    handler
        .projections
        .user_changed(&ctx.cache, new_member.guild_id, new_member.user.id)
        .await;

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
    ctx: Context,
    new: Option<serenity::model::guild::Member>,
    event: serenity::model::event::GuildMemberUpdateEvent,
) {
    if handler.runtime.maintains_caches()
        && let Err(err) = crate::database::guild_cache::sync_live_member_roles(
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
    handler
        .projections
        .user_changed(&ctx.cache, event.guild_id, event.user.id)
        .await;

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
    handler: &Handler,
    _ctx: Context,
    chunk: serenity::model::event::GuildMembersChunkEvent,
) {
    // Chunks list each member's full role set; absence from a chunk says
    // nothing, so only these members' assignments are corrected.
    if handler.runtime.maintains_caches()
        && let Err(err) = crate::database::guild_cache::sync_chunk_member_roles(
            &handler.database,
            chunk.guild_id,
            chunk.members.values(),
        )
        .await
    {
        error!(
            error = %err,
            guild_id = chunk.guild_id.get(),
            "failed to sync member roles from a member chunk"
        );
    }
    handler.projections.chunk_received(&chunk);

    for member in chunk.members.into_values() {
        if member.user.bot {
            continue;
        }
        crate::database::user_names::observe(
            &handler.database,
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
    if handler.runtime.maintains_caches()
        && let Err(err) =
            crate::database::guild_cache::sync_guild_info(&handler.database, &new_guild).await
    {
        error!(
            guild_id = new_guild.id.get(),
            owner_id = new_guild.owner_id.get(),
            "failed to update guild owner, name or icon: {}",
            err
        );
    }
}

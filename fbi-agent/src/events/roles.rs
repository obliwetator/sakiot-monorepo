use serenity::client::Context;
use tracing::error;

use crate::event_handler::Handler;

pub async fn guild_role_create(
    handler: &Handler,
    _ctx: Context,
    new: serenity::model::guild::Role,
) {
    if let Err(err) = crate::database::guild_cache::sync_live_role(&handler.database, &new).await {
        error!(
            error = %err,
            guild_id = new.guild_id.get(),
            role_id = new.id.get(),
            "failed to sync created role"
        );
    }
}

pub async fn guild_role_delete(
    handler: &Handler,
    _ctx: Context,
    guild_id: serenity::model::id::GuildId,
    removed_role_id: serenity::model::id::RoleId,
    _removed_role_data_if_available: Option<serenity::model::guild::Role>,
) {
    if let Err(err) =
        crate::database::guild_cache::delete_live_role(&handler.database, removed_role_id).await
    {
        error!(
            error = %err,
            guild_id = guild_id.get(),
            role_id = removed_role_id.get(),
            "failed to delete role from cache"
        );
    }
}

pub async fn guild_role_update(
    handler: &Handler,
    _ctx: Context,
    _old_data_if_available: Option<serenity::model::guild::Role>,
    new: serenity::model::guild::Role,
) {
    if let Err(err) = crate::database::guild_cache::sync_live_role(&handler.database, &new).await {
        error!(
            error = %err,
            guild_id = new.guild_id.get(),
            role_id = new.id.get(),
            "failed to sync updated role"
        );
    }
}

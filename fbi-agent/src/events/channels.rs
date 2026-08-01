use serenity::client::Context;
use tracing::error;

use crate::event_handler::Handler;

pub async fn category_create() {}

pub async fn category_delete() {}

pub async fn channel_create(
    handler: &Handler,
    _ctx: Context,
    channel: serenity::model::channel::GuildChannel,
) {
    if let Err(err) =
        crate::database::guild_cache::sync_live_channel(&handler.database, &channel).await
    {
        error!(
            error = %err,
            guild_id = channel.guild_id.get(),
            channel_id = channel.id.get(),
            "failed to sync created channel"
        );
    }
}

pub async fn channel_delete(
    handler: &Handler,
    _ctx: Context,
    channel: serenity::model::channel::GuildChannel,
) {
    if let Err(err) =
        crate::database::guild_cache::delete_live_channel(&handler.database, channel.id).await
    {
        error!(
            error = %err,
            guild_id = channel.guild_id.get(),
            channel_id = channel.id.get(),
            "failed to delete channel from cache"
        );
    }
}

pub async fn channel_pins_update() {}

pub async fn channel_update(
    handler: &Handler,
    _ctx: Context,
    channel: serenity::model::channel::GuildChannel,
) {
    if let Err(err) =
        crate::database::guild_cache::sync_live_channel(&handler.database, &channel).await
    {
        error!(
            error = %err,
            guild_id = channel.guild_id.get(),
            channel_id = channel.id.get(),
            "failed to sync updated channel"
        );
    }
}

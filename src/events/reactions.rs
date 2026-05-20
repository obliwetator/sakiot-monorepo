use serenity::client::Context;

use crate::event_handler::Handler;

pub async fn reaction_add(
    _self: &Handler,
    _ctx: Context,
    _add_reaction: serenity::model::channel::Reaction,
) {
}

pub async fn reaction_remove(
    _self: &Handler,
    _ctx: Context,
    _removed_reaction: serenity::model::channel::Reaction,
) {
}

pub async fn reaction_remove_all(
    _self: &Handler,
    _ctx: Context,
    _channel_id: serenity::model::id::ChannelId,
    _removed_from_message_id: serenity::model::id::MessageId,
) {
}

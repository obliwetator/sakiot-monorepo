use serenity::{
    client::Context,
    model::channel::{Message, MessageType},
};
use tracing::{debug, warn};

use crate::event_handler::Handler;

pub async fn message(_self: &Handler, ctx: Context, msg: Message) {
    if _self.runtime.is_draining() {
        return;
    }

    if msg.kind == MessageType::Regular {
        let data_read = ctx.data.read().await;
        if let Some(metrics) = data_read.get::<crate::BotMetricsKey>() {
            metrics
                .messages_received
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        return;
    }
    debug!(kind = ?msg.kind, "unhandled message type");
}

pub async fn message_delete(
    _self: &Handler,
    _ctx: Context,
    _channel_id: serenity::model::id::ChannelId,
    _deleted_message_id: serenity::model::id::MessageId,
    _guild_id: Option<serenity::model::id::GuildId>,
) {
    // Not yet implemented — log and return instead of panicking
    warn!(
        "message_delete not implemented: channel={:?} message={:?} guild={:?}",
        _channel_id, _deleted_message_id, _guild_id
    );
}

pub async fn message_delete_bulk(
    _self: &Handler,
    _ctx: Context,
    _channel_id: serenity::model::id::ChannelId,
    _multiple_deleted_messages_ids: Vec<serenity::model::id::MessageId>,
    _guild_id: Option<serenity::model::id::GuildId>,
) {
    // Not yet implemented — log and return instead of panicking
    warn!(
        "message_delete_bulk not implemented: channel={:?} count={} guild={:?}",
        _channel_id,
        _multiple_deleted_messages_ids.len(),
        _guild_id
    );
}

pub async fn message_update(
    _self: &Handler,
    _ctx: Context,
    _old_if_available: Option<Message>,
    _new: Option<Message>,
    _event: serenity::model::event::MessageUpdateEvent,
) {
    // Not yet implemented — log and return instead of panicking
    warn!("message_update not implemented: event={:?}", _event.id);
}

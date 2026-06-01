use serenity::{
    client::Context,
    model::channel::{Message, MessageType},
};
use tracing::{debug, warn};

use crate::event_handler::Handler;

pub async fn message(_self: &Handler, ctx: Context, msg: Message) {
    // let pool = db_helper::get_pool_from_ctx(&ctx).await;
    // db_helper::get_channels(&pool).await;
    if _self.runtime.is_draining() {
        return;
    }

    match msg.kind {
        MessageType::Regular => {
            let data_read = ctx.data.read().await;
            if let Some(metrics) = data_read.get::<crate::BotMetricsKey>() {
                metrics
                    .messages_received
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let _ = metrics.update_tx.send(());
            }
        }
        MessageType::ChannelFollowAdd => {}
        MessageType::GroupCallCreation => {}
        MessageType::GroupIconUpdate => {}
        MessageType::GroupNameUpdate => {}
        MessageType::GroupRecipientAddition => {}
        MessageType::GroupRecipientRemoval => {}
        MessageType::GuildDiscoveryDisqualified => {}
        MessageType::GuildDiscoveryRequalified => {}
        MessageType::GuildInviteReminder => {}
        MessageType::InlineReply => {}
        MessageType::MemberJoin => {}
        MessageType::NitroBoost => {}
        MessageType::NitroTier1 => {}
        MessageType::NitroTier2 => {}
        MessageType::NitroTier3 => {}
        MessageType::PinsAdd => {}
        MessageType::GuildDiscoveryGracePeriodInitialWarning => {
            debug!("Unhandled message type: GuildDiscoveryGracePeriodInitialWarning");
        }
        MessageType::GuildDiscoveryGracePeriodFinalWarning => {
            debug!("Unhandled message type: GuildDiscoveryGracePeriodFinalWarning");
        }
        MessageType::ThreadCreated => {
            debug!("Unhandled message type: ThreadCreated");
        }
        MessageType::ChatInputCommand => {
            // info!("slash command message: {:#?}", msg);
        }
        MessageType::ThreadStarterMessage => {
            debug!("Unhandled message type: ThreadStarterMessage");
        }
        MessageType::ContextMenuCommand => {
            debug!("Unhandled message type: ContextMenuCommand");
        }
        MessageType::AutoModAction => {
            debug!("Unhandled message type: AutoModAction");
        }
        MessageType::RoleSubscriptionPurchase => {
            debug!("Unhandled message type: RoleSubscriptionPurchase");
        }
        MessageType::InteractionPremiumUpsell => {
            debug!("Unhandled message type: InteractionPremiumUpsell");
        }
        MessageType::StageStart => {
            debug!("Unhandled message type: StageStart");
        }
        MessageType::StageEnd => {
            debug!("Unhandled message type: StageEnd");
        }
        MessageType::StageSpeaker => {
            debug!("Unhandled message type: StageSpeaker");
        }
        MessageType::StageTopic => {
            debug!("Unhandled message type: StageTopic");
        }
        MessageType::GuildApplicationPremiumSubscription => {
            debug!("Unhandled message type: GuildApplicationPremiumSubscription");
        }
        MessageType::Unknown(_) => {
            debug!("unknown message type");
        }
        _ => {
            debug!("unhandled message type");
        }
    }
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

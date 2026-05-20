use serenity::client::Context;

use crate::event_handler::Handler;

pub async fn guild_emojis_update(
    _self: &Handler,
    _ctx: Context,
    _guild_id: serenity::model::id::GuildId,
    _current_state: std::collections::HashMap<
        serenity::model::id::EmojiId,
        serenity::model::guild::Emoji,
    >,
) {

}

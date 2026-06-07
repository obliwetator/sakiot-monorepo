use serenity::client::Context;

use crate::event_handler::Handler;

pub async fn guild_role_create(_self: &Handler, _ctx: Context, _new: serenity::model::guild::Role) {
}

pub async fn guild_role_delete(
    _self: &Handler,
    _ctx: Context,
    _guild_id: serenity::model::id::GuildId,
    _removed_role_id: serenity::model::id::RoleId,
    _removed_role_data_if_available: Option<serenity::model::guild::Role>,
) {
}

pub async fn guild_role_update(
    _self: &Handler,
    _ctx: Context,
    _old_data_if_available: Option<serenity::model::guild::Role>,
    _new: serenity::model::guild::Role,
) {
}

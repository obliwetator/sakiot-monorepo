use serenity::client::Context;

use crate::event_handler::Handler;

pub async fn invite_create(
    _self: &Handler,
    _ctx: Context,
    _data: serenity::model::event::InviteCreateEvent,
) {
}

pub async fn invite_delete(
    _self: &Handler,
    _ctx: Context,
    _data: serenity::model::event::InviteDeleteEvent,
) {
}

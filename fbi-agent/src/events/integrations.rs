use serenity::client::Context;

use crate::event_handler::Handler;

pub async fn guild_integrations_update(
    _self: &Handler,
    _ctx: Context,
    _guild_id: serenity::model::id::GuildId,
) {
}

pub async fn integration_create(
    _self: &Handler,
    _ctx: Context,
    _integration: serenity::model::guild::Integration,
) {
}

pub async fn integration_update(
    _self: &Handler,
    _ctx: Context,
    _integration: serenity::model::guild::Integration,
) {
}

pub async fn integration_delete(
    _self: &Handler,
    _ctx: Context,
    _integration_id: serenity::model::id::IntegrationId,
    _guild_id: serenity::model::id::GuildId,
    _application_id: Option<serenity::model::id::ApplicationId>,
) {
}

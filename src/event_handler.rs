use serenity::{
    async_trait,
    model::{channel::Message, gateway::Ready},
    prelude::*,
};

use sqlx::{Pool, Postgres};
use tracing::info;

use crate::{commands, database, events};

pub struct Handler {
    pub(crate) database: Pool<Postgres>,
    pub(crate) jam_cooldown: crate::cooldown::JamCooldown,
    pub(crate) runtime: std::sync::Arc<crate::runtime::RuntimeState>,
}

impl Handler {
    async fn with_metrics<F>(ctx: &Context, f: F)
    where
        F: FnOnce(&crate::BotMetrics),
    {
        let data = ctx.data.read().await;
        if let Some(metrics) = data.get::<crate::BotMetricsKey>() {
            f(metrics);
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, ctx: Context, guilds: Vec<serenity::model::id::GuildId>) {
        events::cache_ready::cache_ready(self, ctx, guilds).await;
    }

    async fn resume(&self, ctx: Context, _: serenity::model::event::ResumedEvent) {
        Self::with_metrics(&ctx, |m| m.record_gateway_resume()).await;
    }

    async fn channel_pins_update(
        &self,
        _ctx: Context,
        _pin: serenity::model::event::ChannelPinsUpdateEvent,
    ) {
        events::channels::channel_pins_update().await;
    }

    async fn guild_ban_addition(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
        banned_user: serenity::model::prelude::User,
    ) {
        events::guilds::guild_ban_addition(self, ctx, guild_id, banned_user).await;
    }

    async fn guild_ban_removal(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
        unbanned_user: serenity::model::prelude::User,
    ) {
        events::guilds::guild_ban_removal(self, ctx, guild_id, unbanned_user).await;
    }

    async fn guild_create(
        &self,
        ctx: Context,
        guild: serenity::model::guild::Guild,
        is_new: Option<bool>,
    ) {
        events::guilds::guild_create(self, ctx, guild, is_new).await;
    }

    async fn guild_delete(
        &self,
        ctx: Context,
        incomplete: serenity::model::guild::UnavailableGuild,
        full: Option<serenity::model::guild::Guild>,
    ) {
        events::guilds::guild_delete(self, ctx, incomplete, full).await;
    }

    async fn guild_emojis_update(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
        current_state: std::collections::HashMap<
            serenity::model::id::EmojiId,
            serenity::model::guild::Emoji,
        >,
    ) {
        events::emojis::guild_emojis_update(self, ctx, guild_id, current_state).await;
    }

    async fn guild_integrations_update(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
    ) {
        events::integrations::guild_integrations_update(self, ctx, guild_id).await;
    }

    async fn guild_member_addition(
        &self,
        ctx: Context,
        new_member: serenity::model::guild::Member,
    ) {
        events::guilds::guild_member_addition(self, ctx, new_member).await;
    }

    async fn guild_member_removal(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
        user: serenity::model::prelude::User,
        member_data_if_available: Option<serenity::model::guild::Member>,
    ) {
        events::guilds::guild_member_removal(self, ctx, guild_id, user, member_data_if_available)
            .await;
    }

    async fn guild_members_chunk(
        &self,
        ctx: Context,
        chunk: serenity::model::event::GuildMembersChunkEvent,
    ) {
        events::guilds::guild_members_chunk(self, ctx, chunk).await;
    }

    async fn guild_member_update(
        &self,
        ctx: Context,
        old_if_available: Option<serenity::model::guild::Member>,
        new: Option<serenity::model::guild::Member>,
        _event: serenity::model::event::GuildMemberUpdateEvent,
    ) {
        if let Some(member) = new {
            events::guilds::guild_member_update(self, ctx, old_if_available, member).await;
        }
    }

    async fn guild_role_create(&self, ctx: Context, new: serenity::model::guild::Role) {
        events::roles::guild_role_create(self, ctx, new).await;
    }

    async fn guild_role_delete(
        &self,
        ctx: Context,
        guild_id: serenity::model::id::GuildId,
        removed_role_id: serenity::model::id::RoleId,
        removed_role_data_if_available: Option<serenity::model::guild::Role>,
    ) {
        events::roles::guild_role_delete(
            self,
            ctx,
            guild_id,
            removed_role_id,
            removed_role_data_if_available,
        )
        .await;
    }

    async fn guild_role_update(
        &self,
        ctx: Context,
        old_data_if_available: Option<serenity::model::guild::Role>,
        new: serenity::model::guild::Role,
    ) {
        events::roles::guild_role_update(self, ctx, old_data_if_available, new).await;
    }

    async fn guild_update(
        &self,
        ctx: Context,
        old_data_if_available: Option<serenity::model::guild::Guild>,
        new_but_incomplete: serenity::model::guild::PartialGuild,
    ) {
        events::guilds::guild_update(self, ctx, old_data_if_available, new_but_incomplete).await;
    }

    async fn invite_create(&self, ctx: Context, data: serenity::model::event::InviteCreateEvent) {
        events::invites::invite_create(self, ctx, data).await;
    }

    async fn invite_delete(&self, ctx: Context, data: serenity::model::event::InviteDeleteEvent) {
        events::invites::invite_delete(self, ctx, data).await;
    }

    async fn message(&self, ctx: Context, msg: Message) {
        events::messages::message(self, ctx, msg).await;
    }

    async fn message_delete(
        &self,
        ctx: Context,
        channel_id: serenity::model::id::ChannelId,
        deleted_message_id: serenity::model::id::MessageId,
        guild_id: Option<serenity::model::id::GuildId>,
    ) {
        events::messages::message_delete(self, ctx, channel_id, deleted_message_id, guild_id).await;
    }

    async fn message_delete_bulk(
        &self,
        ctx: Context,
        channel_id: serenity::model::id::ChannelId,
        multiple_deleted_messages_ids: Vec<serenity::model::id::MessageId>,
        guild_id: Option<serenity::model::id::GuildId>,
    ) {
        events::messages::message_delete_bulk(
            self,
            ctx,
            channel_id,
            multiple_deleted_messages_ids,
            guild_id,
        )
        .await;
    }

    async fn message_update(
        &self,
        ctx: Context,
        old_if_available: Option<Message>,
        new: Option<Message>,
        event: serenity::model::event::MessageUpdateEvent,
    ) {
        events::messages::message_update(self, ctx, old_if_available, new, event).await;
    }

    async fn reaction_add(&self, ctx: Context, add_reaction: serenity::model::channel::Reaction) {
        events::reactions::reaction_add(self, ctx, add_reaction).await;
    }

    async fn reaction_remove(
        &self,
        ctx: Context,
        removed_reaction: serenity::model::channel::Reaction,
    ) {
        events::reactions::reaction_remove(self, ctx, removed_reaction).await;
    }

    async fn reaction_remove_all(
        &self,
        ctx: Context,
        channel_id: serenity::model::id::ChannelId,
        removed_from_message_id: serenity::model::id::MessageId,
    ) {
        events::reactions::reaction_remove_all(self, ctx, channel_id, removed_from_message_id)
            .await;
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
        database::update_guild_present(ready.guilds, self).await;
        commands::register_global_commands(&ctx).await;
    }

    async fn user_update(
        &self,
        _ctx: Context,
        _old_data: Option<serenity::model::prelude::CurrentUser>,
        new: serenity::model::prelude::CurrentUser,
    ) {
        info!(user_id = %new.id, username = %new.name, "bot user updated");
    }

    async fn voice_server_update(
        &self,
        ctx: Context,
        update: serenity::model::event::VoiceServerUpdateEvent,
    ) {
        events::voice::voice_server_update(self, ctx, update).await;
    }

    async fn voice_state_update(
        &self,
        ctx: Context,
        old: Option<serenity::model::prelude::VoiceState>,
        new: serenity::model::prelude::VoiceState,
    ) {
        Self::with_metrics(&ctx, |m| m.record_voice_state_update()).await;
        events::voice::voice_state_update(self, ctx, old, new).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: serenity::all::Interaction) {
        Self::with_metrics(&ctx, |m| m.record_command_executed()).await;
        events::interactions::interaction_create(self, ctx, interaction).await;
    }

    async fn integration_create(
        &self,
        ctx: Context,
        integration: serenity::model::guild::Integration,
    ) {
        events::integrations::integration_create(self, ctx, integration).await;
    }

    async fn integration_update(
        &self,
        ctx: Context,
        integration: serenity::model::guild::Integration,
    ) {
        events::integrations::integration_update(self, ctx, integration).await;
    }

    async fn integration_delete(
        &self,
        ctx: Context,
        integration_id: serenity::model::id::IntegrationId,
        guild_id: serenity::model::id::GuildId,
        application_id: Option<serenity::model::id::ApplicationId>,
    ) {
        events::integrations::integration_delete(
            self,
            ctx,
            integration_id,
            guild_id,
            application_id,
        )
        .await;
    }
}

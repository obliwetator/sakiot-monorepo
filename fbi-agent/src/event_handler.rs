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

    async fn channel_create(&self, ctx: Context, channel: serenity::model::channel::GuildChannel) {
        events::channels::channel_create(self, ctx, channel).await;
    }

    async fn channel_delete(
        &self,
        ctx: Context,
        channel: serenity::model::channel::GuildChannel,
        _messages: Option<Vec<Message>>,
    ) {
        events::channels::channel_delete(self, ctx, channel).await;
    }

    async fn channel_update(
        &self,
        ctx: Context,
        _old: Option<serenity::model::channel::GuildChannel>,
        new: serenity::model::channel::GuildChannel,
    ) {
        events::channels::channel_update(self, ctx, new).await;
    }

    async fn resume(&self, ctx: Context, _: serenity::model::event::ResumedEvent) {
        Self::with_metrics(&ctx, |m| m.record_gateway_resume()).await;
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
        _old_if_available: Option<serenity::model::guild::Member>,
        new: Option<serenity::model::guild::Member>,
        event: serenity::model::event::GuildMemberUpdateEvent,
    ) {
        events::guilds::guild_member_update(self, ctx, new, event).await;
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

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
        database::update_guild_present(ready.guilds, &self.database).await;
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

    async fn voice_state_update(
        &self,
        ctx: Context,
        old: Option<serenity::model::prelude::VoiceState>,
        new: serenity::model::prelude::VoiceState,
    ) {
        events::voice::voice_state_update(self, ctx, old, new).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: serenity::all::Interaction) {
        events::interactions::interaction_create(self, ctx, interaction).await;
    }
}

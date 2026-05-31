use serenity::client::Context;
use serenity::model::guild::Guild;
use serenity::model::id::GuildId;
use tracing::error;

use crate::event_handler::Handler;
use crate::{database, get_lock_read};

pub async fn cache_ready(handler: &Handler, ctx: Context, guilds: Vec<GuildId>) {
    let guild_cached: Vec<Guild> = guilds
        .iter()
        .filter_map(|guild| {
            guild
                .to_guild_cached(&ctx)
                .map(|g| g.to_owned())
                .or_else(|| {
                    error!("Guild {} missing from cache during cache_ready", guild);
                    None
                })
        })
        .collect();

    {
        // Acquire the write lock once for the entire loop instead of once per
        // iteration. Repeatedly dropping and re-acquiring the write lock lets
        // readers (e.g. get_channel_with_most_members) slip in between iterations
        // and causes unnecessary contention.
        let lock = get_lock_read(&ctx).await;
        let mut guard = lock.write().await;
        for ele in &guild_cached {
            let afk = ele.afk_metadata.as_ref().map(|m| m.afk_channel_id.get());
            guard.insert(ele.id.get(), afk);
        }
    }

    let _ = database::update_info(handler, &ctx, &guilds).await;
    database::user_names::seed_from_guilds(&handler.database, &guild_cached).await;
}

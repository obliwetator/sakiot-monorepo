//! Shared fixture helpers for the integration tests.

use sqlx::PgPool;

/// Membership comes from the bot's roster (`guild_members`), complete once
/// per guild, not from the login snapshot (`user_guilds`). Gives every guild
/// with `user_guilds` rows a complete roster holding exactly those members,
/// as the bot would after a full sync. Call after seeding `user_guilds`.
#[allow(dead_code)] // Not every test binary uses every helper.
pub async fn complete_rosters_from_user_guilds(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO guild_projection_state
            (guild_id, generation, roster_complete_at, presence_synced_at)
         SELECT DISTINCT id, 1, now(), now() FROM user_guilds
         ON CONFLICT (guild_id) DO UPDATE
            SET roster_complete_at = COALESCE(guild_projection_state.roster_complete_at, now())",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, username)
         SELECT id, user_id, 'member-' || user_id FROM user_guilds
         ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Removes a member from the roster, as a member-remove event would.
#[allow(dead_code)]
pub async fn leave_guild(pool: &PgPool, guild_id: i64, user_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM guild_members WHERE guild_id = $1 AND user_id = $2")
        .bind(guild_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

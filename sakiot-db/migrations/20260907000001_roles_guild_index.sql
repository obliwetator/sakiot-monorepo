-- no-transaction
-- get_combined_perm_for_user aggregates roles per guild.
CREATE INDEX CONCURRENTLY IF NOT EXISTS roles_guild_idx
    ON public.roles (guild_id);

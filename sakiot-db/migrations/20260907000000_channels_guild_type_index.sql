-- no-transaction
-- Permission checks resolve a guild's voice channels by (guild_id, type).
-- CONCURRENTLY needs its own transaction-free statement, so each index lives
-- in its own migration file. A failed build leaves an INVALID index: drop it
-- (DROP INDEX CONCURRENTLY) and re-run this migration.
CREATE INDEX CONCURRENTLY IF NOT EXISTS channels_guild_type_idx
    ON public.channels (guild_id, type);

-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS stamps_lookup_idx
    ON public.stamps (guild_id, target_user_id, stamp_ts DESC);

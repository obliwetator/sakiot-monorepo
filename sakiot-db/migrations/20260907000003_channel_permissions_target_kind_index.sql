-- no-transaction
-- Channel overwrites are resolved per target (role or user) and kind.
CREATE INDEX CONCURRENTLY IF NOT EXISTS channel_permissions_target_kind_idx
    ON public.channel_permissions (target_id, kind);

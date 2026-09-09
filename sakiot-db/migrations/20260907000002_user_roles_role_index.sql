-- no-transaction
-- The same aggregation probes user_roles by role_id.
CREATE INDEX CONCURRENTLY IF NOT EXISTS user_roles_role_idx
    ON public.user_roles (role_id);

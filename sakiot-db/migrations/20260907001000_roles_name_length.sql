-- Discord role names may be up to 100 characters; the baseline column allowed
-- 50, so longer names failed the agent's role-cache upsert.
--
-- Widening a varchar is metadata-only (no table rewrite), but it still takes a
-- brief ACCESS EXCLUSIVE lock. The short lock_timeout keeps a busy database
-- from blocking the deploy: the ALTER fails and the migration can be retried
-- instead of holding a lock behind a long-running query.
SET lock_timeout = '5s';

ALTER TABLE public.roles
    ALTER COLUMN name TYPE character varying(100);

RESET lock_timeout;

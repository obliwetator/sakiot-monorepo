-- no-transaction
-- Stamp history is listed newest-first for one target in one guild; the old
-- index led with channel_id, which the listing does not filter on.
DROP INDEX CONCURRENTLY IF EXISTS public.stamps_lookup_idx;

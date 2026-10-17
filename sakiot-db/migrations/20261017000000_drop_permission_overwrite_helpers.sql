-- Since 1.11.3 the web server loads a guild's permission facts in one
-- statement of its own (web-server/src/permissions/facts.rs). These helpers
-- fed the per-check implementation it replaced, which only the permission
-- harness still ran, and no longer does; no release since 1.11.3 calls them.
DROP FUNCTION public.get_roles_overwrites_for_channels_from_user(bigint, bigint);
DROP FUNCTION public.get_user_channel_overriders_for_user_id(bigint, bigint);

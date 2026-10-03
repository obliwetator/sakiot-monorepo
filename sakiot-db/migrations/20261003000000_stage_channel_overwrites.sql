-- The agent records voice (type 2) and stage (type 13) channels, and
-- `web-server` now builds channel visibility for both. These helpers feed a
-- member's role and member overwrites into that computation; while they read
-- only `type = 2`, a stage channel would apply `@everyone` overwrites but drop
-- role and member denies, granting access Discord does not.
CREATE OR REPLACE FUNCTION public.get_roles_overwrites_for_channels_from_user(
    p_target_id bigint,
    p_guild_id bigint
) RETURNS TABLE(allow bigint, deny bigint, channel_id bigint, role_id bigint)
    LANGUAGE sql
    STABLE
AS $$
    SELECT cp.allow, cp.deny, cp.channel_id, cp.target_id
      FROM public.channel_permissions cp
      JOIN public.user_roles ur ON ur.role_id = cp.target_id
      JOIN public.channels c ON c.channel_id = cp.channel_id
     WHERE ur.user_id = p_target_id
       AND cp.kind = 'role'
       AND c.type IN (2, 13)
       AND c.guild_id = p_guild_id
$$;

CREATE OR REPLACE FUNCTION public.get_user_channel_overriders_for_user_id(
    p_target_id bigint,
    p_guild_id bigint
) RETURNS TABLE(allow bigint, deny bigint, channel_id bigint)
    LANGUAGE sql
    STABLE
AS $$
    SELECT cp.allow, cp.deny, cp.channel_id
      FROM public.channel_permissions cp
      JOIN public.channels c ON c.channel_id = cp.channel_id
     WHERE c.type IN (2, 13)
       AND c.guild_id = p_guild_id
       AND cp.kind = 'user'
       AND cp.target_id = p_target_id
$$;

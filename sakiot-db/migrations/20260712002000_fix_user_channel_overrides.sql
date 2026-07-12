-- `channel_permissions.kind` stores member overwrites as `user`. The baseline
-- helper still looked for the pre-schema value `member`, silently ignoring
-- every user-specific allow/deny. Session-wide authorization must honor these
-- overwrites for every audible channel.
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
     WHERE c.type = 2
       AND c.guild_id = p_guild_id
       AND cp.kind = 'user'
       AND cp.target_id = p_target_id
$$;

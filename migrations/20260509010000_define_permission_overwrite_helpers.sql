CREATE OR REPLACE FUNCTION public.get_roles_overwrites_for_channels_from_user(
    p_target_id bigint,
    p_guild_id bigint
)
RETURNS TABLE(allow bigint, deny bigint, channel_id bigint, role_id bigint)
LANGUAGE plpgsql
AS $function$
BEGIN
    RETURN QUERY
    SELECT
        channel_permissions.allow,
        channel_permissions.deny,
        channel_permissions.channel_id,
        channel_permissions.target_id
    FROM channel_permissions
    INNER JOIN user_roles ON channel_permissions.target_id = user_roles.role_id
    INNER JOIN channels ON channel_permissions.channel_id = channels.channel_id
    WHERE user_roles.user_id = p_target_id
        AND channel_permissions.kind = 'role'
        AND channels."type" = 2
        AND channels.guild_id = p_guild_id;
END
$function$;

DROP FUNCTION IF EXISTS public.get_user_channel_overriders_for_user_id(bigint, bigint);

CREATE FUNCTION public.get_user_channel_overriders_for_user_id(
    p_target_id bigint,
    p_guild_id bigint
)
RETURNS TABLE(allow bigint, deny bigint, channel_id bigint)
LANGUAGE plpgsql
AS $function$
BEGIN
    RETURN QUERY
    SELECT
        channel_permissions.allow,
        channel_permissions.deny,
        channel_permissions.channel_id
    FROM channel_permissions
    INNER JOIN channels ON channel_permissions.channel_id = channels.channel_id
    WHERE channels.type = 2
        AND channels.guild_id = p_guild_id
        AND channel_permissions.kind = 'member'
        AND channel_permissions.target_id = p_target_id;
END
$function$;

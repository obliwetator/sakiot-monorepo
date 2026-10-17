-- Realtime: display changes leave the `perm` payload. `web-server` now
-- re-authorizes open sockets on `perm` and tells a viewer only when what they
-- may see changed, so a rename, which changes no one's access, needs its own
-- payload, routed to whatever shows the name:
--   names    {"v":1,"k":"names","g":guild,"r":"guilds"}   name or icon
--            {"v":1,"k":"names","g":guild,"r":"roles"}    a role's name or
--                                                          colours, or the
--                                                          set of roles
--            {"v":1,"k":"names","g":guild,"r":"channels","c":channel}
--                                                          a voice or stage
--                                                          channel's name or
--                                                          type, or the set
--                                                          of them
-- Role membership, which the role list counts, sends `members` as the roster
-- does. `perm` keeps firing for every column authorization reads.

SET lock_timeout = '5s';

CREATE FUNCTION public.realtime_names() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_json jsonb;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_json := to_jsonb(OLD);
    ELSE
        row_json := to_jsonb(NEW);
    END IF;

    CASE TG_TABLE_NAME
        WHEN 'guilds' THEN
            PERFORM public.realtime_notify(jsonb_build_object(
                'v', 1, 'k', 'names', 'g', row_json ->> 'id', 'r', 'guilds'));
        WHEN 'roles' THEN
            PERFORM public.realtime_notify(jsonb_build_object(
                'v', 1, 'k', 'names', 'g', row_json ->> 'guild_id', 'r', 'roles'));
        WHEN 'channels' THEN
            PERFORM public.realtime_notify(jsonb_build_object(
                'v', 1, 'k', 'names', 'g', row_json ->> 'guild_id', 'r', 'channels',
                'c', row_json ->> 'channel_id'));
        ELSE
            NULL;
    END CASE;
    RETURN NULL;
END
$$;

-- A role assignment changes the role list's member counts. When the role is
-- already gone (a cascade from deleting it), its own trigger has spoken.
CREATE FUNCTION public.realtime_role_members() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    guild_id bigint;
BEGIN
    SELECT r.guild_id INTO guild_id
      FROM public.roles r
     WHERE r.role_id = (CASE WHEN TG_OP = 'DELETE' THEN OLD.role_id ELSE NEW.role_id END);
    IF guild_id IS NOT NULL THEN
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'members', 'g', guild_id::text));
    END IF;
    RETURN NULL;
END
$$;

-- guilds: the owner decides authorization; the name and icon only show.
DROP TRIGGER realtime_update ON public.guilds;
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guilds
    FOR EACH ROW
    WHEN (OLD.owner_id IS DISTINCT FROM NEW.owner_id)
    EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_names_update
    AFTER UPDATE ON public.guilds
    FOR EACH ROW
    WHEN ((OLD.name, OLD.icon) IS DISTINCT FROM (NEW.name, NEW.icon))
    EXECUTE FUNCTION public.realtime_names();

-- roles: the permission bits decide authorization; the name and colours only
-- show, and the role list shows every role.
DROP TRIGGER realtime_update ON public.roles;
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.roles
    FOR EACH ROW
    WHEN ((OLD.guild_id, OLD.role_id, OLD.permission)
          IS DISTINCT FROM (NEW.guild_id, NEW.role_id, NEW.permission))
    EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_names_insert_delete
    AFTER INSERT OR DELETE ON public.roles
    FOR EACH ROW EXECUTE FUNCTION public.realtime_names();
CREATE TRIGGER realtime_names_update
    AFTER UPDATE ON public.roles
    FOR EACH ROW
    WHEN ((OLD.name, OLD.color, OLD.color_secondary, OLD.color_tertiary)
          IS DISTINCT FROM (NEW.name, NEW.color, NEW.color_secondary, NEW.color_tertiary))
    EXECUTE FUNCTION public.realtime_names();

CREATE TRIGGER realtime_members
    AFTER INSERT OR DELETE ON public.user_roles
    FOR EACH ROW EXECUTE FUNCTION public.realtime_role_members();

-- channels: the type decides which channels count (voice and stage); the
-- name only shows, and only voice and stage channels are shown anywhere.
DROP TRIGGER realtime_update ON public.channels;
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.channels
    FOR EACH ROW
    WHEN ((OLD.channel_id, OLD.guild_id, OLD.type)
          IS DISTINCT FROM (NEW.channel_id, NEW.guild_id, NEW.type))
    EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_names_insert
    AFTER INSERT ON public.channels
    FOR EACH ROW
    WHEN (NEW.type IN (2, 13))
    EXECUTE FUNCTION public.realtime_names();
CREATE TRIGGER realtime_names_delete
    AFTER DELETE ON public.channels
    FOR EACH ROW
    WHEN (OLD.type IN (2, 13))
    EXECUTE FUNCTION public.realtime_names();
CREATE TRIGGER realtime_names_update
    AFTER UPDATE ON public.channels
    FOR EACH ROW
    WHEN ((OLD.name, OLD.type) IS DISTINCT FROM (NEW.name, NEW.type)
          AND (OLD.type IN (2, 13) OR NEW.type IN (2, 13)))
    EXECUTE FUNCTION public.realtime_names();

RESET lock_timeout;

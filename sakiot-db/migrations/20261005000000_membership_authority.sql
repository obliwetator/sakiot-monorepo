-- Membership authority (realtime phase 4): the bot's roster
-- (`guild_members`), not the guild list Discord returned at login
-- (`user_guilds`), decides who belongs to a guild. See `membership` in
-- `web-server/src/permissions.rs`.

SET lock_timeout = '5s';

-- The guild picker shows live names and icons instead of the login snapshot.
-- NULL until the bot next syncs the guild.
ALTER TABLE public.guilds
    ADD COLUMN name text,
    ADD COLUMN icon text;

-- Background media jobs re-check access when they run. A dev login (local,
-- staging and preview builds only) follows the seeded-guild rules, so the job
-- records which kind of login requested it.
ALTER TABLE public.media_jobs
    ADD COLUMN requester_dev boolean NOT NULL DEFAULT false;
ALTER TABLE public.composition_jobs
    ADD COLUMN requester_dev boolean NOT NULL DEFAULT false;

-- Joining or leaving a guild, and a roster becoming complete, now change
-- authorization: send the guild-wide `perm` payload so open sockets are
-- re-authorized (the `members` and `presence` payloads stay as they are).
CREATE OR REPLACE FUNCTION public.realtime_permissions() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_json jsonb;
    guild_id bigint;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_json := to_jsonb(OLD);
    ELSE
        row_json := to_jsonb(NEW);
    END IF;

    CASE TG_TABLE_NAME
        WHEN 'guilds', 'user_guilds' THEN
            guild_id := (row_json ->> 'id')::bigint;
        WHEN 'roles', 'channels', 'guild_members', 'guild_projection_state' THEN
            guild_id := (row_json ->> 'guild_id')::bigint;
        WHEN 'user_roles' THEN
            SELECT r.guild_id INTO guild_id
              FROM public.roles r
             WHERE r.role_id = (row_json ->> 'role_id')::bigint;
        WHEN 'channel_permissions' THEN
            SELECT c.guild_id INTO guild_id
              FROM public.channels c
             WHERE c.channel_id = (row_json ->> 'channel_id')::bigint;
        ELSE
            guild_id := NULL;
    END CASE;

    IF guild_id IS NULL THEN
        PERFORM public.realtime_notify(jsonb_build_object('v', 1, 'k', 'perm'));
    ELSE
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'perm', 'g', guild_id::text));
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_membership
    AFTER INSERT OR DELETE ON public.guild_members
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();

CREATE TRIGGER realtime_membership_insert_delete
    AFTER INSERT OR DELETE ON public.guild_projection_state
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_membership_update
    AFTER UPDATE ON public.guild_projection_state
    FOR EACH ROW
    WHEN ((OLD.roster_complete_at IS NULL) IS DISTINCT FROM (NEW.roster_complete_at IS NULL))
    EXECUTE FUNCTION public.realtime_permissions();

-- A renamed guild or new icon refreshes the guild picker (`access_changed`
-- invalidates it), alongside an owner change.
DROP TRIGGER realtime_update ON public.guilds;
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guilds
    FOR EACH ROW
    WHEN ((OLD.owner_id, OLD.name, OLD.icon) IS DISTINCT FROM (NEW.owner_id, NEW.name, NEW.icon))
    EXECUTE FUNCTION public.realtime_permissions();

RESET lock_timeout;

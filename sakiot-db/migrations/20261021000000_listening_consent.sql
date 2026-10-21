-- Listening consent: each member decides, per guild, who may hear the
-- recordings of their voice. `shared` (no row, the default) means whoever the
-- guild's permissions let listen; `private` (a row) means only themselves,
-- whatever anyone holds, the owner included. It applies at read time to every
-- recording, mix track, clip and stamp of their voice, past ones included, and
-- is never copied onto them. Changing it is never gated: former members may
-- too.
--
-- Realtime: a change re-authorizes the guild's viewers like any permission
-- change (`perm`), and refreshes the member's own setting:
--   privacy {"v":1,"k":"privacy","g":guild,"u":user}

SET lock_timeout = '5s';

CREATE TABLE public.recording_private (
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, user_id)
);

COMMENT ON TABLE public.recording_private IS
    'Members who keep the recordings of their voice in a guild to themselves (listening consent "private"); no row means shared.';

CREATE FUNCTION public.realtime_recording_private() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.recording_private;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'perm', 'g', row_data.guild_id::text));
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'privacy',
        'g', row_data.guild_id::text,
        'u', row_data.user_id::text));
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.recording_private
    FOR EACH ROW EXECUTE FUNCTION public.realtime_recording_private();

RESET lock_timeout;

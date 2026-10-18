-- Former members: someone no longer in a guild keeps reaching the
-- recordings of their own voice there, and nothing else. Everyone is one
-- once the bot has left the guild, whose cached roles and overwrites then
-- stop updating.
--
-- Every permission check asks whether a viewer who does not belong has
-- recordings in the guild, so that question needs an index of its own (the
-- one on (guild_id, user_id, started_at) covers open sessions only).
--
-- `guilds.bot_left_at` records when the bot left, for a shorter retention
-- of abandoned guilds later; nothing reads it yet. `guilds_present` keeps it:
-- a removal sets it, a return clears it. A change of presence changes who
-- belongs, so it re-authorizes the guild's viewers like any permission
-- change.

SET lock_timeout = '5s';

CREATE INDEX recording_sessions_guild_user_idx
    ON public.recording_sessions (guild_id, user_id);

ALTER TABLE public.guilds ADD COLUMN bot_left_at timestamp with time zone;

-- Guilds the bot had already left, at the latest now.
UPDATE public.guilds g
   SET bot_left_at = now()
 WHERE NOT EXISTS (SELECT 1 FROM public.guilds_present p WHERE p.guild_id = g.id);

COMMENT ON COLUMN public.guilds.bot_left_at IS
    'When the bot left the guild (at the latest; NULL while it is there). Kept by guilds_present''s trigger.';

CREATE FUNCTION public.guilds_present_changed() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    guild_id bigint;
BEGIN
    IF TG_OP = 'DELETE' THEN
        guild_id := OLD.guild_id;
        UPDATE public.guilds SET bot_left_at = now() WHERE id = guild_id;
    ELSE
        guild_id := NEW.guild_id;
        UPDATE public.guilds
           SET bot_left_at = NULL
         WHERE id = guild_id AND bot_left_at IS NOT NULL;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'perm', 'g', guild_id::text));
    RETURN NULL;
END
$$;

CREATE TRIGGER bot_presence
    AFTER INSERT OR DELETE ON public.guilds_present
    FOR EACH ROW EXECUTE FUNCTION public.guilds_present_changed();

RESET lock_timeout;

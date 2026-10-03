-- Member and voice presence projections (realtime phase 3).
--
-- `guild_members` is the current roster and `voice_presence` who is in which
-- voice or stage channel, both written by `fbi-agent` from its gateway cache.
-- `guild_projection_state` says which bot instance writes a guild's
-- projections: every write locks the guild's row and checks the owner and
-- generation, so a stale writer (an old release still draining, or a
-- superseded snapshot) is fenced out. A replacement claims a guild only once
-- it has a complete snapshot, by bumping the generation.
--
-- These tables are not authoritative for access: `user_guilds` still decides
-- membership until the membership authority switch (phase 4).

SET lock_timeout = '5s';

CREATE TABLE public.guild_projection_state (
    guild_id bigint PRIMARY KEY,
    -- NULL once the owner stopped without a replacement: projections are
    -- then unknown, not empty.
    owner_instance_id text,
    generation bigint NOT NULL DEFAULT 0,
    -- When the owner last wrote a complete roster (a full member list from
    -- GUILD_CREATE or a full chunk set). NULL: never complete.
    roster_complete_at timestamp with time zone,
    -- When the owner last wrote every voice state of the guild.
    presence_synced_at timestamp with time zone,
    updated_at timestamp with time zone NOT NULL DEFAULT now()
);

CREATE TABLE public.guild_members (
    guild_id bigint NOT NULL
        REFERENCES public.guild_projection_state (guild_id) ON DELETE CASCADE,
    user_id bigint NOT NULL,
    username text NOT NULL,
    global_name text,
    nickname text,
    is_bot boolean NOT NULL DEFAULT false,
    PRIMARY KEY (guild_id, user_id)
);

CREATE TABLE public.voice_presence (
    guild_id bigint NOT NULL
        REFERENCES public.guild_projection_state (guild_id) ON DELETE CASCADE,
    user_id bigint NOT NULL,
    channel_id bigint NOT NULL,
    self_mute boolean NOT NULL DEFAULT false,
    self_deaf boolean NOT NULL DEFAULT false,
    server_mute boolean NOT NULL DEFAULT false,
    server_deaf boolean NOT NULL DEFAULT false,
    streaming boolean NOT NULL DEFAULT false,
    video boolean NOT NULL DEFAULT false,
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX voice_presence_channel_idx ON public.voice_presence (guild_id, channel_id);

-- Realtime payloads (see 20261003010000_realtime_notifications.sql):
--   members  {"v":1,"k":"members","g":guild}   the roster changed
--   presence {"v":1,"k":"presence","g":guild}  who is in voice changed
-- Both are guild-only: every subscriber refetches its own filtered view, so
-- nobody learns about a channel they cannot view.

CREATE FUNCTION public.realtime_voice_presence() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    guild_id bigint;
BEGIN
    IF TG_OP = 'DELETE' THEN
        guild_id := OLD.guild_id;
    ELSE
        guild_id := NEW.guild_id;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'presence', 'g', guild_id::text));
    RETURN NULL;
END
$$;

-- A roster change also changes presence when it renames someone in voice.
CREATE FUNCTION public.realtime_guild_members() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.guild_members;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'members', 'g', row_data.guild_id::text));
    IF EXISTS (
        SELECT 1 FROM public.voice_presence vp
         WHERE vp.guild_id = row_data.guild_id AND vp.user_id = row_data.user_id
    ) THEN
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'presence', 'g', row_data.guild_id::text));
    END IF;
    RETURN NULL;
END
$$;

-- Ownership and completeness decide whether projections are known at all.
CREATE FUNCTION public.realtime_guild_projection_state() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    guild_id bigint;
BEGIN
    IF TG_OP = 'DELETE' THEN
        guild_id := OLD.guild_id;
    ELSE
        guild_id := NEW.guild_id;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'members', 'g', guild_id::text));
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'presence', 'g', guild_id::text));
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.voice_presence
    FOR EACH ROW EXECUTE FUNCTION public.realtime_voice_presence();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.voice_presence
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_voice_presence();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guild_members
    FOR EACH ROW EXECUTE FUNCTION public.realtime_guild_members();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_members
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_guild_members();

-- A re-claim by the same owner or a refreshed timestamp changes nothing a
-- viewer sees; only a change of owner or of completeness does.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guild_projection_state
    FOR EACH ROW EXECUTE FUNCTION public.realtime_guild_projection_state();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_projection_state
    FOR EACH ROW
    WHEN ((OLD.owner_instance_id, OLD.roster_complete_at IS NULL,
           OLD.presence_synced_at IS NULL)
          IS DISTINCT FROM
          (NEW.owner_instance_id, NEW.roster_complete_at IS NULL,
           NEW.presence_synced_at IS NULL))
    EXECUTE FUNCTION public.realtime_guild_projection_state();

RESET lock_timeout;

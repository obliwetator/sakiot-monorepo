-- Realtime dashboard: committed writes signal `web-server` through NOTIFY on
-- one channel, `sakiot_realtime`. Payloads carry identifiers and a version
-- only; `web-server` re-reads everything through its authorized endpoints.
--
-- Payload kinds (ids are strings, so JavaScript never rounds them):
--   session   {"v":1,"k":"session","g":guild,"s":session,"c":channel}
--   recording {"v":1,"k":"recording","g":guild}   a file without a session
--   clip      {"v":1,"k":"clip","g":guild,"id":clip,"c":channel,"s":session}
--   stamp     {"v":1,"k":"stamp","g":guild,"id":stamp,"c":channel,"s":session}
--   opt_out   {"v":1,"k":"opt_out","g":guild,"u":user}
--   settings  {"v":1,"k":"settings","g":guild,"r":table}
--   perm      {"v":1,"k":"perm","g":guild} or {"v":1,"k":"perm"} when the
--             guild cannot be resolved (after a cascade), meaning resync all
--
-- Rules:
-- * Update triggers fire only when a displayed column changes, so no-op
--   writes, heartbeats (`audio_files.recording_heartbeat_at`, written every
--   10 s while recording) and bookkeeping columns send nothing.
-- * Permission payloads are guild-only, and PostgreSQL drops duplicate
--   payloads within a transaction, so a full guild sync sends one
--   notification however many rows it writes.
-- * The bodies only build JSON from the row and look up a guild by primary
--   key; they cannot fail for a valid row. They run inside the recorder's
--   transactions, where an error would fail the recording write.

SET lock_timeout = '5s';

CREATE FUNCTION public.realtime_notify(payload jsonb) RETURNS void
    LANGUAGE sql
AS $$
    SELECT pg_notify('sakiot_realtime', payload::text)
$$;

CREATE FUNCTION public.realtime_recording_sessions() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.recording_sessions;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'session',
        'g', row_data.guild_id::text,
        's', row_data.id::text,
        'c', row_data.starting_channel_id::text));
    RETURN NULL;
END
$$;

CREATE FUNCTION public.realtime_audio_files() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.audio_files;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;

    IF row_data.recording_session_id IS NULL THEN
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'recording', 'g', row_data.guild_id::text));
    ELSE
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'session',
            'g', row_data.guild_id::text,
            's', row_data.recording_session_id::text,
            'c', row_data.channel_id::text));
    END IF;

    -- A fragment moved between sessions (or out of one) changes both.
    IF TG_OP = 'UPDATE'
       AND OLD.recording_session_id IS DISTINCT FROM NEW.recording_session_id THEN
        IF OLD.recording_session_id IS NULL THEN
            PERFORM public.realtime_notify(jsonb_build_object(
                'v', 1, 'k', 'recording', 'g', OLD.guild_id::text));
        ELSE
            PERFORM public.realtime_notify(jsonb_build_object(
                'v', 1, 'k', 'session',
                'g', OLD.guild_id::text,
                's', OLD.recording_session_id::text,
                'c', OLD.channel_id::text));
        END IF;
    END IF;
    RETURN NULL;
END
$$;

CREATE FUNCTION public.realtime_clips() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.clips;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    -- Listings only ever show clips of one guild; a guild-less clip has no
    -- audience.
    IF row_data.guild_id IS NOT NULL THEN
        PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
            'v', 1, 'k', 'clip',
            'g', row_data.guild_id::text,
            'id', row_data.clip_id,
            'c', row_data.channel_id::text,
            's', row_data.recording_session_id::text)));
    END IF;
    RETURN NULL;
END
$$;

CREATE FUNCTION public.realtime_stamps() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.stamps;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
        'v', 1, 'k', 'stamp',
        'g', row_data.guild_id::text,
        'id', row_data.id::text,
        'c', row_data.channel_id::text,
        's', row_data.recording_session_id::text)));
    RETURN NULL;
END
$$;

CREATE FUNCTION public.realtime_recording_opt_outs() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.recording_opt_outs;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'opt_out',
        'g', row_data.guild_id::text,
        'u', row_data.user_id::text));
    RETURN NULL;
END
$$;

-- Shared by the four guild settings tables, which all have `guild_id`.
CREATE FUNCTION public.realtime_settings() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    guild_id bigint;
BEGIN
    IF TG_OP = 'DELETE' THEN
        guild_id := (to_jsonb(OLD) ->> 'guild_id')::bigint;
    ELSE
        guild_id := (to_jsonb(NEW) ->> 'guild_id')::bigint;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'settings',
        'g', guild_id::text,
        'r', TG_TABLE_NAME));
    RETURN NULL;
END
$$;

-- Permission and membership cache tables. `user_roles` and
-- `channel_permissions` have no guild column, so it is looked up through
-- `roles` and `channels`; when those rows are already gone (a cascade from a
-- deleted role or channel), a guild-less payload asks for a full resync.
CREATE FUNCTION public.realtime_permissions() RETURNS trigger
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
        WHEN 'roles', 'channels' THEN
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

-- recording_sessions: creation, state, timestamps, pause and resume, deletion.
-- Bookkeeping (updated_at, owner_instance_id, deadlines) is not displayed.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.recording_sessions
    FOR EACH ROW EXECUTE FUNCTION public.realtime_recording_sessions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.recording_sessions
    FOR EACH ROW
    WHEN ((OLD.state, OLD.ended_at, OLD.pause_started_at, OLD.resumed_at,
           OLD.deletion_requested_at, OLD.end_reason, OLD.current_channel_id)
          IS DISTINCT FROM
          (NEW.state, NEW.ended_at, NEW.pause_started_at, NEW.resumed_at,
           NEW.deletion_requested_at, NEW.end_reason, NEW.current_channel_id))
    EXECUTE FUNCTION public.realtime_recording_sessions();

-- audio_files: new fragments, finalization, reaping, silence removal and
-- moves between sessions; never the recording heartbeat alone.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.audio_files
    FOR EACH ROW EXECUTE FUNCTION public.realtime_audio_files();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.audio_files
    FOR EACH ROW
    WHEN ((OLD.end_ts, OLD.reaped, OLD.silence, OLD.channel_id,
           OLD.recording_session_id, OLD.segment_index, OLD.file_name)
          IS DISTINCT FROM
          (NEW.end_ts, NEW.reaped, NEW.silence, NEW.channel_id,
           NEW.recording_session_id, NEW.segment_index, NEW.file_name))
    EXECUTE FUNCTION public.realtime_audio_files();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.clips
    FOR EACH ROW EXECUTE FUNCTION public.realtime_clips();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.clips
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_clips();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.stamps
    FOR EACH ROW EXECUTE FUNCTION public.realtime_stamps();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.stamps
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_stamps();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.recording_opt_outs
    FOR EACH ROW EXECUTE FUNCTION public.realtime_recording_opt_outs();

-- Settings: `updated_at` and `updated_by` change on every save, even one
-- that stores the same values, so only the settings themselves count.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guild_voice_settings
    FOR EACH ROW EXECUTE FUNCTION public.realtime_settings();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_voice_settings
    FOR EACH ROW
    WHEN (OLD.pending_cap_seconds IS DISTINCT FROM NEW.pending_cap_seconds)
    EXECUTE FUNCTION public.realtime_settings();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guild_recording_policy
    FOR EACH ROW EXECUTE FUNCTION public.realtime_settings();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_recording_policy
    FOR EACH ROW
    WHEN ((OLD.retention_days, OLD.excluded_channel_ids)
          IS DISTINCT FROM (NEW.retention_days, NEW.excluded_channel_ids))
    EXECUTE FUNCTION public.realtime_settings();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guild_jam_cooldowns
    FOR EACH ROW EXECUTE FUNCTION public.realtime_settings();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_jam_cooldowns
    FOR EACH ROW
    WHEN (OLD.cooldown_seconds IS DISTINCT FROM NEW.cooldown_seconds)
    EXECUTE FUNCTION public.realtime_settings();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.user_jam_cooldown_overrides
    FOR EACH ROW EXECUTE FUNCTION public.realtime_settings();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.user_jam_cooldown_overrides
    FOR EACH ROW
    WHEN (OLD.cooldown_seconds IS DISTINCT FROM NEW.cooldown_seconds)
    EXECUTE FUNCTION public.realtime_settings();

-- Permission and membership caches. The agent writes only real differences
-- (one transaction per guild), so these fire only on actual changes. Role and
-- channel renames count too: the dashboard shows those names.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.guilds
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guilds
    FOR EACH ROW
    WHEN (OLD.owner_id IS DISTINCT FROM NEW.owner_id)
    EXECUTE FUNCTION public.realtime_permissions();

-- `user_guilds` is rewritten at every OAuth login; only membership (insert,
-- delete) and the owner flag affect authorization.
CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.user_guilds
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.user_guilds
    FOR EACH ROW
    WHEN (OLD.owner IS DISTINCT FROM NEW.owner)
    EXECUTE FUNCTION public.realtime_permissions();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.roles
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.roles
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_permissions();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.user_roles
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.channels
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.channels
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION public.realtime_permissions();

CREATE TRIGGER realtime_insert_delete
    AFTER INSERT OR DELETE ON public.channel_permissions
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permissions();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.channel_permissions
    FOR EACH ROW
    WHEN ((OLD.kind, OLD.allow, OLD.deny) IS DISTINCT FROM (NEW.kind, NEW.allow, NEW.deny))
    EXECUTE FUNCTION public.realtime_permissions();

RESET lock_timeout;

-- Keeping recordings past retention, within limits the bot's operators set.
--
-- `operators` run this Sakiot deployment. Being one is not a Discord role
-- and nothing a guild can grant: they are added on the server with
-- `web_server operators add <user id>`, and only they set `guild_limits`.
--
-- `guild_limits.kept_recordings` caps how many distinct recordings a guild
-- keeps, 0 without a row. Within it, holders of `manage_settings` set how
-- many each member may keep (`keep_allowances`): the member's own
-- allowance, else the largest of their roles', else @everyone's, else only
-- the guild's cap.
--
-- `kept_recordings` is one member's keep of a session. Retention never
-- removes a kept session. Any other removal (a manager's, or its speaker's
-- erasure) drops the session's keeps, so every kept session is visible.
--
-- A member keeps a session retention removed by restoring it. So a soft
-- deletion job now records the clips it hid (`hidden_clip_ids`; NULL on
-- older jobs, whose restoration brings back only the session). A
-- restoration deletes the job, and `recording_restorations` is its audit,
-- under the job's id.
--
-- Realtime: changes to keeps, allowances and limits refresh every member's
-- keeping page,
--   settings {"v":1,"k":"settings","g":guild,"r":<the table's name>}

SET lock_timeout = '5s';

CREATE TABLE public.operators (
    user_id bigint PRIMARY KEY,
    added_at timestamp with time zone NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.operators IS
    'The people who run this Sakiot deployment: they set every guild''s limits. Managed with `web_server operators`.';

CREATE TABLE public.guild_limits (
    guild_id bigint PRIMARY KEY,
    kept_recordings integer NOT NULL CHECK (kept_recordings BETWEEN 0 AND 100000),
    updated_by bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT now()
);

CREATE TABLE public.keep_allowances (
    guild_id bigint NOT NULL,
    subject_kind text NOT NULL,
    subject_id bigint,
    allowance integer NOT NULL CHECK (allowance BETWEEN 0 AND 100000),
    updated_by bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT keep_allowances_subject CHECK (
        subject_kind IN ('everyone', 'role', 'user')
        AND (subject_kind = 'everyone') = (subject_id IS NULL)),
    CONSTRAINT keep_allowances_one_per_subject
        UNIQUE NULLS NOT DISTINCT (guild_id, subject_kind, subject_id)
);

CREATE TABLE public.kept_recordings (
    recording_session_id bigint NOT NULL
        REFERENCES public.recording_sessions (id) ON DELETE CASCADE,
    user_id bigint NOT NULL,
    guild_id bigint NOT NULL,
    kept_at timestamp with time zone NOT NULL DEFAULT now(),
    PRIMARY KEY (recording_session_id, user_id)
);
CREATE INDEX kept_recordings_keeper_idx ON public.kept_recordings (guild_id, user_id);

ALTER TABLE public.recording_deletion_jobs ADD COLUMN hidden_clip_ids text[];

CREATE TABLE public.recording_restorations (
    id text PRIMARY KEY,
    guild_id bigint NOT NULL,
    recording_session_id bigint NOT NULL,
    reason text NOT NULL CHECK (reason IN ('manager', 'retention')),
    removed_by bigint,
    removed_at timestamp with time zone NOT NULL,
    restored_by bigint NOT NULL,
    restored_at timestamp with time zone NOT NULL DEFAULT now(),
    kept boolean NOT NULL
);

COMMENT ON TABLE public.recording_restorations IS
    'A removed session brought back, by a manager or by a member keeping it: the deleted deletion job''s audit, under its id.';

CREATE FUNCTION public.realtime_keeping() RETURNS trigger
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
        'v', 1, 'k', 'settings',
        'g', guild_id::text,
        'r', TG_TABLE_NAME));
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_change
    AFTER INSERT OR UPDATE OR DELETE ON public.guild_limits
    FOR EACH ROW EXECUTE FUNCTION public.realtime_keeping();
CREATE TRIGGER realtime_change
    AFTER INSERT OR UPDATE OR DELETE ON public.keep_allowances
    FOR EACH ROW EXECUTE FUNCTION public.realtime_keeping();
CREATE TRIGGER realtime_change
    AFTER INSERT OR UPDATE OR DELETE ON public.kept_recordings
    FOR EACH ROW EXECUTE FUNCTION public.realtime_keeping();

RESET lock_timeout;

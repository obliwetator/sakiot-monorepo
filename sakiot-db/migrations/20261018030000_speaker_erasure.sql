-- Erasure by the speaker: anyone may erase a finalized session of their own
-- voice, or a clip holding it. The request hides it from everyone but them
-- at once; they can undo it for 30 days, and then it is purged for good.
--
-- `recording_erasures` is the request, kept apart from the deletion job
-- (one per session): a manager's or retention's removal keeps its own job
-- and reason, and undo needs the record of exactly what this request hid.
-- `hid_session` is true only if the request tombstoned the session, which
-- was then visible; `hidden_clip_ids` are the clips it hid, each visible
-- before. Undo reverts those and nothing else. Once `purge_at` passes, the
-- worker converts the erasure into a permanent deletion job (reason
-- `speaker`), upgrading a session's existing job in place. Rows are never
-- deleted: they are the speaker's side of the audit trail.
--
-- A clip has had no purge of its own, only soft deletion. A deletion job
-- now names a session or a clip; a clip's job is always permanent.
--
-- Realtime: an erasure payload refreshes the speaker's listings,
--   erasure {"v":1,"k":"erasure","g":guild,"u":speaker,"s":session,"id":clip,
--            "h":"1" when the request hid the session}
-- and, when it hid a session, other viewers' stamps of it, which undo shows
-- again without a session payload saying so. The job events of a speaker's
-- deletion reach only them (`u` on `job`).

SET lock_timeout = '5s';

CREATE TABLE public.recording_erasures (
    id text PRIMARY KEY,
    guild_id bigint NOT NULL,
    recording_session_id bigint,
    clip_id text,
    speaker_id bigint NOT NULL,
    requested_at timestamp with time zone NOT NULL DEFAULT now(),
    purge_at timestamp with time zone NOT NULL,
    state text NOT NULL DEFAULT 'grace'
        CHECK (state IN ('grace', 'cancelled', 'converted')),
    hid_session boolean NOT NULL DEFAULT false,
    hidden_clip_ids text[] NOT NULL DEFAULT '{}',
    deletion_job_id text,
    finished_at timestamp with time zone,
    CHECK (num_nonnulls(recording_session_id, clip_id) = 1),
    CHECK (clip_id IS NULL OR NOT hid_session),
    CHECK ((state = 'grace') = (finished_at IS NULL))
);
CREATE UNIQUE INDEX recording_erasures_open_session_idx
    ON public.recording_erasures (recording_session_id) WHERE state <> 'cancelled';
CREATE UNIQUE INDEX recording_erasures_open_clip_idx
    ON public.recording_erasures (clip_id) WHERE state <> 'cancelled';
CREATE INDEX recording_erasures_due_idx
    ON public.recording_erasures (purge_at) WHERE state = 'grace';

COMMENT ON TABLE public.recording_erasures IS
    'A speaker''s erasure of their own session or of a clip holding their voice: hidden for a grace period, undoable, then converted into a permanent deletion job.';

ALTER TABLE public.recording_deletion_jobs
    ALTER COLUMN recording_session_id DROP NOT NULL,
    ADD COLUMN clip_id text UNIQUE,
    ADD CONSTRAINT recording_deletion_jobs_subject_check
        CHECK (num_nonnulls(recording_session_id, clip_id) = 1),
    ADD CONSTRAINT recording_deletion_jobs_clip_permanent_check
        CHECK (clip_id IS NULL OR mode = 'permanent'),
    DROP CONSTRAINT recording_deletion_jobs_reason_check;
ALTER TABLE public.recording_deletion_jobs
    ADD CONSTRAINT recording_deletion_jobs_reason_check
        CHECK (reason IN ('manager', 'retention', 'speaker'));

CREATE OR REPLACE FUNCTION public.realtime_jobs() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_json jsonb := to_jsonb(NEW);
BEGIN
    -- A media job without a guild has no subscribed audience.
    IF row_json ->> 'guild_id' IS NOT NULL THEN
        PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
            'v', 1, 'k', 'job',
            'r', TG_ARGV[0],
            'g', row_json ->> 'guild_id',
            'id', row_json ->> 'id',
            'u', CASE
                     WHEN TG_ARGV[0] = 'composition' THEN row_json ->> 'user_id'
                     WHEN TG_ARGV[0] = 'deletion' AND row_json ->> 'reason' = 'speaker'
                         THEN row_json ->> 'requested_by'
                 END)));
    END IF;
    RETURN NULL;
END
$$;

CREATE FUNCTION public.realtime_erasures() RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
        'v', 1, 'k', 'erasure',
        'g', NEW.guild_id::text,
        'u', NEW.speaker_id::text,
        's', NEW.recording_session_id::text,
        'id', NEW.clip_id,
        'h', CASE WHEN NEW.hid_session THEN '1' END)));
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_insert
    AFTER INSERT ON public.recording_erasures
    FOR EACH ROW EXECUTE FUNCTION public.realtime_erasures();
CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.recording_erasures
    FOR EACH ROW
    WHEN (OLD.state IS DISTINCT FROM NEW.state)
    EXECUTE FUNCTION public.realtime_erasures();

RESET lock_timeout;

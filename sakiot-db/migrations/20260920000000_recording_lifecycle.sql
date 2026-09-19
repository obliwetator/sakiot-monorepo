-- Guild recording controls are opt-in. NULL retention means no automatic purge.
CREATE TABLE public.guild_recording_policy (
    guild_id bigint PRIMARY KEY,
    retention_days integer CHECK (retention_days BETWEEN 1 AND 3650),
    excluded_channel_ids bigint[] NOT NULL DEFAULT '{}',
    updated_by bigint,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT excluded_channels_positive CHECK (
        array_position(excluded_channel_ids, NULL) IS NULL
    )
);

ALTER TABLE public.recording_sessions
    ADD COLUMN deletion_requested_at timestamptz;
CREATE INDEX recording_sessions_retention_idx
    ON public.recording_sessions (guild_id, ended_at, id)
    WHERE state = 'finalized' AND deletion_requested_at IS NULL;

-- This row is also the durable audit record: it survives removal of the
-- session and contains only identifiers, actor, reason, stage and outcome.
CREATE TABLE public.recording_deletion_jobs (
    id text PRIMARY KEY,
    recording_session_id bigint NOT NULL UNIQUE,
    guild_id bigint NOT NULL,
    requested_by bigint,
    reason text NOT NULL CHECK (reason IN ('manager', 'retention')),
    state text NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'ready', 'failed')),
    stage text NOT NULL DEFAULT 'queued',
    attempts integer NOT NULL DEFAULT 0,
    attempt_token text,
    lease_expires_at timestamptz,
    retry_at timestamptz NOT NULL DEFAULT now(),
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    CHECK ((state = 'running') = (attempt_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);
CREATE INDEX recording_deletion_jobs_claim_idx
    ON public.recording_deletion_jobs (retry_at, created_at)
    WHERE state IN ('queued', 'running');

-- A clip render can be in flight when a manager requests deletion. Serialize
-- publication with the tombstone and reject any late session/fragment or
-- composed derivative. The rendering endpoint then discards its output.
CREATE FUNCTION public.reject_deleted_recording_clip() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(NEW.guild_id);
    IF (NEW.recording_session_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM recording_sessions rs
             WHERE rs.id=NEW.recording_session_id AND rs.deletion_requested_at IS NOT NULL
        )) OR EXISTS (
            SELECT 1 FROM audio_files af JOIN recording_sessions rs ON rs.id=af.recording_session_id
             WHERE af.guild_id=NEW.guild_id AND af.channel_id=NEW.channel_id
               AND af.file_name=NEW.original_file_name AND rs.deletion_requested_at IS NOT NULL
        ) OR (NEW.composition IS NOT NULL AND EXISTS (
            SELECT 1 FROM jsonb_array_elements(COALESCE(NEW.composition->'segments','[]'::jsonb)) segment
              JOIN clips source ON source.clip_id=segment->>'source_id'
             WHERE source.guild_id=NEW.guild_id AND source.deleted_at IS NOT NULL
        )) THEN
        RAISE EXCEPTION 'recording deletion in progress' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER clips_reject_deleted_recording
    BEFORE INSERT OR UPDATE OF recording_session_id, original_file_name, composition ON public.clips
    FOR EACH ROW EXECUTE FUNCTION public.reject_deleted_recording_clip();

-- An overwritten composition can still have older archive versions derived
-- from a recording. Preserve source identity across editor-job cleanup so a
-- later recording deletion can purge the entire destination clip prefix.
CREATE TABLE public.clip_source_history (
    target_clip_id text NOT NULL,
    source_clip_id text NOT NULL,
    PRIMARY KEY (target_clip_id, source_clip_id)
);
CREATE INDEX clip_source_history_source_idx ON public.clip_source_history (source_clip_id);

CREATE FUNCTION public.track_clip_source_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.composition IS NOT NULL THEN
        INSERT INTO public.clip_source_history (target_clip_id, source_clip_id)
        SELECT NEW.clip_id, segment->>'source_id'
          FROM jsonb_array_elements(
              CASE WHEN jsonb_typeof(NEW.composition->'segments')='array'
                   THEN NEW.composition->'segments' ELSE '[]'::jsonb END
          ) segment
         WHERE segment->>'source_id' IS NOT NULL
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER clips_track_source_history
    AFTER INSERT OR UPDATE OF composition ON public.clips
    FOR EACH ROW EXECUTE FUNCTION public.track_clip_source_history();

INSERT INTO public.clip_source_history (target_clip_id, source_clip_id)
SELECT c.clip_id, segment->>'source_id'
  FROM public.clips c,
       LATERAL jsonb_array_elements(
           CASE WHEN jsonb_typeof(c.composition->'segments')='array'
                THEN c.composition->'segments' ELSE '[]'::jsonb END
       ) segment
 WHERE segment->>'source_id' IS NOT NULL
ON CONFLICT DO NOTHING;
INSERT INTO public.clip_source_history (target_clip_id, source_clip_id)
SELECT j.result_clip_id, segment->>'source_id'
  FROM public.composition_jobs j,
       LATERAL jsonb_array_elements(
           CASE WHEN jsonb_typeof(j.snapshot->'body'->'segments')='array'
                THEN j.snapshot->'body'->'segments' ELSE '[]'::jsonb END
       ) segment
 WHERE segment->>'source_id' IS NOT NULL
ON CONFLICT DO NOTHING;

-- The original migration reflected the earlier no-delete key. Keep migration
-- history immutable and correct the live catalog comment here.
COMMENT ON TABLE public.media_objects IS
    'Durable B2 archive queue and verification ledger. Media credentials may permanently delete versions; recording lifecycle deletion is audited and fenced.';

-- Logical recordings and serialized voice handoff audit data.
--
-- This migration is deliberately additive. `audio_files.recording_session_id`
-- remains nullable so an older draining FBI-agent release can keep inserting
-- fragments during a rolling deploy. A later rollout may validate the FK and
-- enforce NOT NULL after all older releases have drained.

CREATE TABLE public.recording_sessions (
    id bigserial PRIMARY KEY,
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    starting_channel_id bigint NOT NULL,
    current_channel_id bigint,
    state text NOT NULL,
    started_at timestamp with time zone NOT NULL,
    ended_at timestamp with time zone,
    pause_started_at timestamp with time zone,
    resumed_at timestamp with time zone,
    pending_deadline_at timestamp with time zone,
    absolute_cap_deadline_at timestamp with time zone,
    next_fragment_start_at timestamp with time zone,
    pending_reason text,
    pending_from_channel_id bigint,
    pending_to_channel_id bigint,
    owner_instance_id text,
    end_reason text,
    last_segment_index integer NOT NULL DEFAULT -1,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    updated_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT recording_sessions_state_check
        CHECK (state = ANY (ARRAY['active', 'pending', 'finalized']::text[])),
    CONSTRAINT recording_sessions_end_after_start_check
        CHECK (ended_at IS NULL OR ended_at >= started_at),
    CONSTRAINT recording_sessions_pending_deadline_check
        CHECK (
            pending_deadline_at IS NULL
            OR pause_started_at IS NULL
            OR pending_deadline_at >= pause_started_at
        ),
    CONSTRAINT recording_sessions_cap_deadline_check
        CHECK (
            absolute_cap_deadline_at IS NULL
            OR pause_started_at IS NULL
            OR absolute_cap_deadline_at >= pause_started_at
        ),
    CONSTRAINT recording_sessions_owner_instance_id_fkey
        FOREIGN KEY (owner_instance_id)
        REFERENCES public.bot_instances(instance_id)
        ON DELETE SET NULL
);

CREATE INDEX recording_sessions_guild_started_idx
    ON public.recording_sessions (guild_id, started_at DESC);

CREATE INDEX recording_sessions_user_open_idx
    ON public.recording_sessions (guild_id, user_id, started_at DESC)
    WHERE state <> 'finalized';

CREATE INDEX recording_sessions_pending_deadline_idx
    ON public.recording_sessions (pending_deadline_at)
    WHERE state = 'pending' AND pending_deadline_at IS NOT NULL;

CREATE INDEX recording_sessions_cap_deadline_idx
    ON public.recording_sessions (absolute_cap_deadline_at)
    WHERE state = 'pending' AND absolute_cap_deadline_at IS NOT NULL;

CREATE INDEX recording_sessions_owner_idx
    ON public.recording_sessions (owner_instance_id)
    WHERE owner_instance_id IS NOT NULL AND state <> 'finalized';

CREATE TABLE public.recording_gaps (
    id bigserial PRIMARY KEY,
    recording_session_id bigint NOT NULL,
    started_at timestamp with time zone NOT NULL,
    ended_at timestamp with time zone NOT NULL,
    reason text NOT NULL,
    from_channel_id bigint,
    to_channel_id bigint,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT recording_gaps_session_fkey
        FOREIGN KEY (recording_session_id)
        REFERENCES public.recording_sessions(id)
        ON DELETE CASCADE,
    CONSTRAINT recording_gaps_end_after_start_check
        CHECK (ended_at >= started_at)
);

CREATE INDEX recording_gaps_session_time_idx
    ON public.recording_gaps (recording_session_id, started_at);

CREATE TABLE public.recording_session_events (
    id bigserial PRIMARY KEY,
    recording_session_id bigint NOT NULL,
    occurred_at timestamp with time zone NOT NULL DEFAULT now(),
    event_type text NOT NULL,
    channel_id bigint,
    previous_channel_id bigint,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    CONSTRAINT recording_session_events_session_fkey
        FOREIGN KEY (recording_session_id)
        REFERENCES public.recording_sessions(id)
        ON DELETE CASCADE
);

CREATE INDEX recording_session_events_session_time_idx
    ON public.recording_session_events (recording_session_id, occurred_at, id);

CREATE TABLE public.voice_connection_events (
    id bigserial PRIMARY KEY,
    operation_id text NOT NULL,
    guild_id bigint NOT NULL,
    owner_instance_id text,
    release_id text,
    trigger text NOT NULL,
    started_at timestamp with time zone NOT NULL,
    completed_at timestamp with time zone NOT NULL,
    from_channel_id bigint,
    to_channel_id bigint,
    population_snapshot jsonb NOT NULL DEFAULT '[]'::jsonb,
    outcome text NOT NULL,
    error text,
    fallback_outcome text,
    fallback_error text,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT voice_connection_events_time_check
        CHECK (completed_at >= started_at),
    CONSTRAINT voice_connection_events_owner_instance_id_fkey
        FOREIGN KEY (owner_instance_id)
        REFERENCES public.bot_instances(instance_id)
        ON DELETE SET NULL
);

CREATE INDEX voice_connection_events_guild_time_idx
    ON public.voice_connection_events (guild_id, started_at DESC, id DESC);

CREATE INDEX voice_connection_events_owner_time_idx
    ON public.voice_connection_events (owner_instance_id, started_at DESC)
    WHERE owner_instance_id IS NOT NULL;

CREATE TABLE public.guild_voice_settings (
    guild_id bigint PRIMARY KEY,
    pending_cap_seconds integer NOT NULL,
    updated_by bigint,
    updated_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT guild_voice_settings_pending_cap_check
        CHECK (pending_cap_seconds >= 60)
);

ALTER TABLE public.audio_files
    ADD COLUMN IF NOT EXISTS recording_session_id bigint,
    ADD COLUMN IF NOT EXISTS segment_index integer;

ALTER TABLE public.voice_state_events
    ADD COLUMN IF NOT EXISTS previous_channel_id bigint;

-- Every existing physical file becomes its own logical recording. Do not infer
-- relationships between historical files. Pre-allocating session ids provides
-- a set-based, deterministic mapping without retaining a legacy mapping column.
CREATE TEMP TABLE logical_recording_backfill_map (
    audio_file_id bigint PRIMARY KEY,
    recording_session_id bigint NOT NULL
) ON COMMIT DROP;

INSERT INTO logical_recording_backfill_map (audio_file_id, recording_session_id)
SELECT af.id, nextval(pg_get_serial_sequence('public.recording_sessions', 'id'))
  FROM public.audio_files af
 WHERE af.recording_session_id IS NULL;

INSERT INTO public.recording_sessions (
    id,
    guild_id,
    user_id,
    starting_channel_id,
    current_channel_id,
    state,
    started_at,
    ended_at,
    owner_instance_id,
    end_reason,
    last_segment_index,
    created_at,
    updated_at
)
SELECT m.recording_session_id,
       af.guild_id,
       af.user_id,
       af.channel_id,
       af.channel_id,
       CASE WHEN af.end_ts IS NULL THEN 'active' ELSE 'finalized' END,
       to_timestamp(COALESCE(af.start_ts, 0)::double precision / 1000.0),
       CASE
           WHEN af.end_ts IS NULL THEN NULL
           ELSE to_timestamp(af.end_ts::double precision / 1000.0)
       END,
       af.recording_owner_instance_id,
       CASE WHEN af.end_ts IS NULL THEN NULL ELSE 'historical_backfill' END,
       0,
       to_timestamp(COALESCE(af.start_ts, 0)::double precision / 1000.0),
       COALESCE(
           CASE
               WHEN af.end_ts IS NULL THEN NULL
               ELSE to_timestamp(af.end_ts::double precision / 1000.0)
           END,
           now()
       )
  FROM public.audio_files af
  JOIN logical_recording_backfill_map m ON m.audio_file_id = af.id;

UPDATE public.audio_files af
   SET recording_session_id = m.recording_session_id,
       segment_index = 0
  FROM logical_recording_backfill_map m
 WHERE m.audio_file_id = af.id;

INSERT INTO public.recording_session_events (
    recording_session_id,
    occurred_at,
    event_type,
    channel_id,
    details
)
SELECT m.recording_session_id,
       to_timestamp(COALESCE(af.start_ts, 0)::double precision / 1000.0),
       'fragment_open',
       af.channel_id,
       jsonb_build_object('audio_file_id', af.id, 'segment_index', 0, 'backfilled', true)
  FROM public.audio_files af
  JOIN logical_recording_backfill_map m ON m.audio_file_id = af.id;

INSERT INTO public.recording_session_events (
    recording_session_id,
    occurred_at,
    event_type,
    channel_id,
    details
)
SELECT m.recording_session_id,
       to_timestamp(af.end_ts::double precision / 1000.0),
       'fragment_close',
       af.channel_id,
       jsonb_build_object('audio_file_id', af.id, 'segment_index', 0, 'backfilled', true)
  FROM public.audio_files af
  JOIN logical_recording_backfill_map m ON m.audio_file_id = af.id
 WHERE af.end_ts IS NOT NULL;

ALTER TABLE public.audio_files
    ADD CONSTRAINT audio_files_recording_session_id_fkey
    FOREIGN KEY (recording_session_id)
    REFERENCES public.recording_sessions(id)
    ON DELETE RESTRICT
    NOT VALID;

ALTER TABLE public.audio_files
    ADD CONSTRAINT audio_files_segment_index_check
    CHECK (segment_index IS NULL OR segment_index >= 0)
    NOT VALID;

CREATE UNIQUE INDEX audio_files_session_segment_idx
    ON public.audio_files (recording_session_id, segment_index)
    WHERE recording_session_id IS NOT NULL AND segment_index IS NOT NULL;

CREATE INDEX audio_files_session_time_idx
    ON public.audio_files (recording_session_id, start_ts, id)
    WHERE recording_session_id IS NOT NULL;

COMMENT ON COLUMN public.audio_files.recording_session_id IS
    'Nullable during rolling deploy; logical parent for channel-bound physical fragment.';

COMMENT ON COLUMN public.audio_files.segment_index IS
    'Zero-based physical fragment order within a logical recording session.';

COMMENT ON TABLE public.recording_gaps IS
    'Explicit synthetic-silence intervals. Permission redaction may extend this model later; current APIs deny the whole session if any audible channel is inaccessible.';

-- Tighten FBI-agent/web shared DB contract.
--
-- Clean break: audio_files.id becomes the primary key used by FKs and runtime
-- updates. file_name remains unique and stable for URLs and file lookups.

ALTER TABLE public.audio_files
    ALTER COLUMN id SET NOT NULL,
    ALTER COLUMN file_name SET NOT NULL;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = 'public.audio_files'::regclass
           AND conname = 'audio_files_pkey'
    )
    AND NOT EXISTS (
        SELECT 1
          FROM pg_constraint c
          JOIN pg_attribute a
            ON a.attrelid = c.conrelid
           AND a.attname = 'id'
         WHERE c.conrelid = 'public.audio_files'::regclass
           AND c.conname = 'audio_files_pkey'
           AND c.conkey = ARRAY[a.attnum]::smallint[]
    ) THEN
        ALTER TABLE public.audio_files
            DROP CONSTRAINT audio_files_pkey;
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = 'public.audio_files'::regclass
           AND conname = 'audio_files_file_name_key'
    ) THEN
        ALTER TABLE public.audio_files
            ADD CONSTRAINT audio_files_file_name_key UNIQUE (file_name);
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = 'public.audio_files'::regclass
           AND conname = 'audio_files_pkey'
    ) THEN
        ALTER TABLE public.audio_files
            ADD CONSTRAINT audio_files_pkey PRIMARY KEY (id);
    END IF;
END $$;

ALTER TABLE public.audio_files
    DROP CONSTRAINT IF EXISTS audio_files_end_after_start_check;
ALTER TABLE public.audio_files
    ADD CONSTRAINT audio_files_end_after_start_check
    CHECK (end_ts IS NULL OR start_ts IS NULL OR end_ts >= start_ts) NOT VALID;

CREATE INDEX IF NOT EXISTS audio_files_live_by_guild_idx
    ON public.audio_files (guild_id, channel_id, file_name)
    WHERE end_ts IS NULL AND reaped IS FALSE;

CREATE INDEX IF NOT EXISTS audio_files_active_recording_lookup_idx
    ON public.audio_files (user_id, guild_id, channel_id, start_ts DESC)
    WHERE end_ts IS NULL AND reaped IS FALSE;

CREATE INDEX IF NOT EXISTS audio_files_live_owner_heartbeat_idx
    ON public.audio_files (recording_owner_instance_id, recording_heartbeat_at)
    WHERE end_ts IS NULL AND reaped IS FALSE;

CREATE INDEX IF NOT EXISTS audio_files_file_name_idx
    ON public.audio_files (file_name);

CREATE INDEX IF NOT EXISTS voice_state_events_guild_time_desc_idx
    ON public.voice_state_events (guild_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS voice_connection_events_guild_time
    ON public.voice_connection_events (guild_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS stamps_guild_channel_stamp_desc_idx
    ON public.stamps (guild_id, channel_id, stamp_ts DESC);

ALTER TABLE public.guild_jam_cooldowns
    DROP CONSTRAINT IF EXISTS guild_jam_cooldowns_nonnegative_check;
ALTER TABLE public.guild_jam_cooldowns
    ADD CONSTRAINT guild_jam_cooldowns_nonnegative_check
    CHECK (cooldown_seconds >= 0) NOT VALID;

ALTER TABLE public.user_jam_cooldown_overrides
    DROP CONSTRAINT IF EXISTS user_jam_cooldown_overrides_nonnegative_check;
ALTER TABLE public.user_jam_cooldown_overrides
    ADD CONSTRAINT user_jam_cooldown_overrides_nonnegative_check
    CHECK (cooldown_seconds >= 0) NOT VALID;

ALTER TABLE public.voice_connection_events
    DROP CONSTRAINT IF EXISTS voice_connection_events_event_type_check;
ALTER TABLE public.voice_connection_events
    ADD CONSTRAINT voice_connection_events_event_type_check
    CHECK (
        event_type = ANY (ARRAY[
            'join',
            'rejoin',
            'switch',
            'join_skipped',
            'join_failed',
            'leave',
            'leave_skipped',
            'leave_failed',
            'switch_start',
            'switch_failed'
        ]::text[])
    ) NOT VALID;

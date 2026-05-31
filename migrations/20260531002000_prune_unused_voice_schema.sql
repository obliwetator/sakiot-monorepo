ALTER TABLE public.audio_files
    DROP CONSTRAINT IF EXISTS state;

ALTER TABLE public.audio_files
    DROP COLUMN IF EXISTS state_enter,
    DROP COLUMN IF EXISTS state_leave,
    DROP COLUMN IF EXISTS recording_session_id,
    DROP COLUMN IF EXISTS last_ssrc;

DROP TABLE IF EXISTS public.audio_files_state;
DROP TABLE IF EXISTS public.guild_channels;
DROP TABLE IF EXISTS public.kpi_test;
DROP TABLE IF EXISTS public.users;

DO $$
BEGIN
    IF to_regclass('public.voice_events_audit') IS NOT NULL THEN
        DROP TABLE IF EXISTS public.voice_events CASCADE;

        ALTER TABLE public.voice_events_audit RENAME TO voice_events;

        IF to_regclass('public.voice_events_audit_id_seq') IS NOT NULL THEN
            ALTER SEQUENCE public.voice_events_audit_id_seq RENAME TO voice_events_id_seq;
        END IF;

        IF EXISTS (
            SELECT 1
            FROM pg_constraint
            WHERE conname = 'voice_events_audit_pkey'
              AND conrelid = 'public.voice_events'::regclass
        ) THEN
            ALTER TABLE public.voice_events
                RENAME CONSTRAINT voice_events_audit_pkey TO voice_events_pkey;
        END IF;

        IF EXISTS (
            SELECT 1
            FROM pg_constraint
            WHERE conname = 'voice_events_audit_event_type_id_fkey'
              AND conrelid = 'public.voice_events'::regclass
        ) THEN
            ALTER TABLE public.voice_events
                RENAME CONSTRAINT voice_events_audit_event_type_id_fkey TO voice_events_event_type_id_fkey;
        END IF;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'voice_events'
          AND column_name = 'timestamp'
    ) THEN
        ALTER TABLE public.voice_events RENAME COLUMN "timestamp" TO occurred_at;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS voice_events_guild_time
    ON public.voice_events (guild_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS voice_events_user_time
    ON public.voice_events (user_id, occurred_at DESC);

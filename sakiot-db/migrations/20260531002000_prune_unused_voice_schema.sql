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
        -- The superseded table is renamed, not dropped: the audit rows are
        -- cheap to keep and the rename is reversible. Only an already-renamed
        -- legacy table (a re-run) falls back to dropping it.
        IF to_regclass('public.voice_events') IS NOT NULL THEN
            IF to_regclass('public.voice_events_legacy') IS NULL THEN
                ALTER TABLE public.voice_events RENAME TO voice_events_legacy;

                -- Dependent object names do not follow a table rename, and the
                -- audit table below reuses the voice_events_* names.
                IF to_regclass('public.voice_events_id_seq') IS NOT NULL THEN
                    ALTER SEQUENCE public.voice_events_id_seq
                        RENAME TO voice_events_legacy_id_seq;
                END IF;

                IF EXISTS (
                    SELECT 1
                    FROM pg_constraint
                    WHERE conname = 'voice_events_pkey'
                      AND conrelid = 'public.voice_events_legacy'::regclass
                ) THEN
                    ALTER TABLE public.voice_events_legacy
                        RENAME CONSTRAINT voice_events_pkey TO voice_events_legacy_pkey;
                END IF;

                IF to_regclass('public.voice_events_channel_time') IS NOT NULL THEN
                    ALTER INDEX public.voice_events_channel_time
                        RENAME TO voice_events_legacy_channel_time;
                END IF;

                IF to_regclass('public.voice_events_user_time') IS NOT NULL THEN
                    ALTER INDEX public.voice_events_user_time
                        RENAME TO voice_events_legacy_user_time;
                END IF;
            ELSE
                DROP TABLE public.voice_events CASCADE;
            END IF;
        END IF;

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

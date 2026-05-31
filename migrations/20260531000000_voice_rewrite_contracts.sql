-- Voice rewrite compatibility contracts.
--
-- These additions are intentionally additive so older readers keep working
-- while FBI-agent moves voice lifecycle into explicit session/recording
-- components.

INSERT INTO public.voice_event_types (id, name)
VALUES
    (1, 'writer_open'),
    (2, 'writer_close'),
    (3, 'writer_error'),
    (4, 'zombie_reaped')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

INSERT INTO public.voice_state_event_types (id, name)
VALUES
    (1, 'server_mute'),
    (2, 'server_unmute'),
    (3, 'server_deafen'),
    (4, 'server_undeafen'),
    (5, 'self_mute'),
    (6, 'self_unmute'),
    (7, 'self_deafen'),
    (8, 'self_undeafen'),
    (9, 'suppress_on'),
    (10, 'suppress_off'),
    (11, 'stream_start'),
    (12, 'stream_stop'),
    (13, 'video_on'),
    (14, 'video_off'),
    (15, 'channel_join'),
    (16, 'channel_leave'),
    (17, 'channel_switch'),
    (18, 'recording_pause'),
    (19, 'recording_resume'),
    (20, 'user_recording_pause'),
    (21, 'user_recording_resume')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

ALTER TABLE public.audio_files
    ADD COLUMN IF NOT EXISTS recording_session_id text,
    ADD COLUMN IF NOT EXISTS finalize_reason text,
    ADD COLUMN IF NOT EXISTS last_ssrc bigint;

CREATE TABLE IF NOT EXISTS public.voice_connection_events (
    id bigserial PRIMARY KEY,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL,
    guild_id bigint NOT NULL,
    channel_id bigint,
    owner_instance_id text,
    event_type text NOT NULL,
    reason text,
    details text
);

CREATE INDEX IF NOT EXISTS voice_connection_events_guild_time
    ON public.voice_connection_events (guild_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS voice_connection_events_owner_time
    ON public.voice_connection_events (owner_instance_id, occurred_at DESC)
    WHERE owner_instance_id IS NOT NULL;

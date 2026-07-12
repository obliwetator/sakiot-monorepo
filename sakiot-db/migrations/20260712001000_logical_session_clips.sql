-- Clips composed across logical recording fragments need the same all-channel
-- authorization as their source session. Nullable keeps legacy clips fully
-- compatible.

ALTER TABLE public.clips
    ADD COLUMN IF NOT EXISTS recording_session_id bigint;

ALTER TABLE public.clips
    ADD CONSTRAINT clips_recording_session_id_fkey
    FOREIGN KEY (recording_session_id)
    REFERENCES public.recording_sessions(id)
    ON DELETE SET NULL
    NOT VALID;

CREATE INDEX clips_recording_session_idx
    ON public.clips (recording_session_id)
    WHERE recording_session_id IS NOT NULL;

ALTER TABLE public.clips
    ADD COLUMN IF NOT EXISTS silence_free boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN public.clips.silence_free IS
    'True when start_time is an offset into the compressed silence-free session timeline.';

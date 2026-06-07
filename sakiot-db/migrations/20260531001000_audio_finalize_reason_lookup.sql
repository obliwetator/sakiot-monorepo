CREATE TABLE IF NOT EXISTS public.audio_file_finalize_reasons (
    id integer PRIMARY KEY,
    reason text NOT NULL UNIQUE
);

INSERT INTO public.audio_file_finalize_reasons (id, reason)
VALUES
    (1, 'writer_close'),
    (2, 'writer_error'),
    (3, 'zombie_reaped'),
    (4, 'file_create'),
    (5, 'writer_init'),
    (6, 'unknown')
ON CONFLICT (id) DO UPDATE SET reason = EXCLUDED.reason;

ALTER TABLE public.audio_files
    ADD COLUMN IF NOT EXISTS finalize_reason_id integer;

UPDATE public.audio_files af
   SET finalize_reason_id = afr.id
  FROM public.audio_file_finalize_reasons afr
 WHERE af.finalize_reason_id IS NULL
   AND af.finalize_reason = afr.reason;

ALTER TABLE public.audio_files
    DROP COLUMN IF EXISTS finalize_reason;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conname = 'audio_files_finalize_reason_id_fkey'
    ) THEN
        ALTER TABLE public.audio_files
            ADD CONSTRAINT audio_files_finalize_reason_id_fkey
            FOREIGN KEY (finalize_reason_id)
            REFERENCES public.audio_file_finalize_reasons(id);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS audio_files_finalize_reason_idx
    ON public.audio_files (finalize_reason_id)
    WHERE finalize_reason_id IS NOT NULL;

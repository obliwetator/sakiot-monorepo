-- Keep the logical recording association on the stamp itself. The physical
-- audio file remains useful for diagnostics and legacy playback, but it is an
-- implementation detail of a recording session and may be removed later.
ALTER TABLE public.stamps
    ADD COLUMN recording_session_id bigint;

UPDATE public.stamps stamp
   SET recording_session_id = audio.recording_session_id
  FROM public.audio_files audio
 WHERE audio.id = stamp.audio_file_id
   AND stamp.recording_session_id IS NULL
   AND audio.recording_session_id IS NOT NULL;

ALTER TABLE ONLY public.stamps
    ADD CONSTRAINT stamps_recording_session_id_fkey
    FOREIGN KEY (recording_session_id)
    REFERENCES public.recording_sessions(id)
    ON DELETE SET NULL;

CREATE INDEX stamps_by_recording_session
    ON public.stamps (recording_session_id)
    WHERE recording_session_id IS NOT NULL;

COMMENT ON COLUMN public.stamps.recording_session_id IS
    'Logical recording session containing this stamp; audio_file_id identifies the physical fragment.';

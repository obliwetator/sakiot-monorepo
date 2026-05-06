ALTER TABLE public.audio_files
ADD COLUMN waveform_end_ts bigint;

COMMENT ON COLUMN public.audio_files.waveform_end_ts IS
'Final end_ts used to generate cached waveform .dat; NULL means no finalized waveform cache.';

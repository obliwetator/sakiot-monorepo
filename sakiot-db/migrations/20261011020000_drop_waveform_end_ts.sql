-- Waveforms are drawn from peaks files since 1.11.1, and a recording's peaks
-- cover whatever the recorder wrote, so no cached waveform has to be marked
-- as built from the final recording. No release since 1.11.1 reads or writes
-- the column. live_audio_files selects af.*, so it is recreated unchanged
-- around the drop. Dropping the column is quick but needs an exclusive lock on
-- audio_files, which the recorder writes constantly; give up rather than queue
-- every recorder write behind it.
SET lock_timeout = '5s';

DROP VIEW public.live_audio_files;
ALTER TABLE public.audio_files DROP COLUMN waveform_end_ts;

-- The recordings still being written: unfinished, with a recorder actor and
-- an owner instance that both heartbeat, and the instance not stopped. Every
-- liveness check reads this view; a view is inlined into each query, which a
-- function with subqueries is not. 120 s is heartbeat::STALE_AFTER_SECONDS.
-- The recorder row names the instance it runs in, the file's owner.
CREATE VIEW public.live_audio_files AS
SELECT af.*
  FROM public.audio_files af
 WHERE af.end_ts IS NULL
   AND af.reaped IS FALSE
   AND EXISTS (
        SELECT 1
          FROM public.recorder_heartbeats rh
          JOIN public.bot_instances bi ON bi.instance_id = rh.owner_instance_id
         WHERE rh.recorder_id = af.recorder_id
           AND rh.owner_instance_id = af.recording_owner_instance_id
           AND rh.heartbeat_at > now() - interval '120 seconds'
           AND bi.heartbeat_at > now() - interval '120 seconds'
           AND bi.state <> 'stopped'
   );

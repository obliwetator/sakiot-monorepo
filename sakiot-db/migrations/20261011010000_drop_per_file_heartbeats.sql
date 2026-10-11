-- Every running agent now writes recorder heartbeats
-- (20261011000000_recorder_heartbeats.sql), so drop the per-file heartbeat
-- and the view's branch for files that only had one. Unfinished files from
-- before that migration stop counting as live and are reaped at the next
-- agent start. Dropping the column also drops its two indexes
-- (audio_files_recording_heartbeat_idx, audio_files_live_owner_heartbeat_idx).
DROP VIEW public.live_audio_files;
ALTER TABLE public.audio_files DROP COLUMN recording_heartbeat_at;

-- A stopping instance closes its own unfinished recordings; the dropped
-- owner index served that lookup.
CREATE INDEX audio_files_unfinished_owner_idx
    ON public.audio_files (recording_owner_instance_id)
    WHERE end_ts IS NULL;

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

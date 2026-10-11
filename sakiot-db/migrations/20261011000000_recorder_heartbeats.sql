-- Recording liveness at the granularity it is produced. Each guild's recorder
-- actor heartbeats every 10 s, so instead of rewriting every live audio_files
-- row (two indexes, never a HOT update; seconds per beat at tens of thousands
-- of recordings), each actor keeps one row here, and every file it creates
-- carries its actor's id. A file stays tied to the process that writes it: a
-- crashed actor's row goes stale, and a restarted process (same instance id)
-- starts new actors with new ids.
CREATE TABLE public.recorder_heartbeats (
    recorder_id text PRIMARY KEY,
    owner_instance_id text NOT NULL,
    guild_id bigint NOT NULL,
    heartbeat_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE public.audio_files ADD COLUMN recorder_id text;

-- The recordings still being written: unfinished, with a recorder actor and
-- an owner instance that both heartbeat, and the instance not stopped. Every
-- liveness check reads this view; a view is inlined into each query, which a
-- function with subqueries is not. 120 s is heartbeat::STALE_AFTER_SECONDS.
--
-- Files from before this migration have no recorder_id and keep their
-- per-file heartbeat until every agent that writes one has stopped; a later
-- migration removes that branch and recording_heartbeat_at.
CREATE VIEW public.live_audio_files AS
SELECT af.*
  FROM public.audio_files af
 WHERE af.end_ts IS NULL
   AND af.reaped IS FALSE
   AND (
        EXISTS (
            SELECT 1
              FROM public.recorder_heartbeats rh
             WHERE rh.recorder_id = af.recorder_id
               AND rh.heartbeat_at > now() - interval '120 seconds'
        )
        OR (
            af.recorder_id IS NULL
            AND af.recording_heartbeat_at > now() - interval '120 seconds'
        )
   )
   AND EXISTS (
        SELECT 1
          FROM public.bot_instances bi
         WHERE bi.instance_id = af.recording_owner_instance_id
           AND bi.heartbeat_at > now() - interval '120 seconds'
           AND bi.state <> 'stopped'
   );

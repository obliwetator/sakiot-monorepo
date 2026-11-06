-- Clips outlive retention: it hides a recording and later deletes it, but no
-- longer touches the clips cut from it. Show the clips earlier retention
-- removals hid, unless an erasure or another removal also hides them. Removals
-- from before `hidden_clip_ids` existed did not list theirs.
SET lock_timeout = '5s';

UPDATE public.clips c
   SET deleted_at = NULL
  FROM public.recording_deletion_jobs j
 WHERE j.reason = 'retention'
   AND j.mode = 'soft'
   AND c.deleted_at IS NOT NULL
   AND (c.clip_id = ANY(j.hidden_clip_ids)
        OR (j.hidden_clip_ids IS NULL
            AND c.recording_session_id = j.recording_session_id))
   AND NOT EXISTS (SELECT 1
                     FROM public.recording_erasures e
                    WHERE e.state <> 'cancelled'
                      AND (e.clip_id = c.clip_id
                           OR c.clip_id = ANY(e.hidden_clip_ids)))
   AND NOT EXISTS (SELECT 1
                     FROM public.recording_deletion_jobs other
                    WHERE other.id <> j.id
                      AND c.clip_id = ANY(other.hidden_clip_ids));

UPDATE public.recording_deletion_jobs
   SET hidden_clip_ids = '{}'
 WHERE reason = 'retention' AND mode = 'soft';

RESET lock_timeout;

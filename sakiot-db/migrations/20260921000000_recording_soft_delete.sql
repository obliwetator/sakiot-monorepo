-- Keep deletion reversible by default. Existing jobs were created by the
-- permanent-deletion implementation; never silently reinterpret or resume them.
ALTER TABLE public.recording_deletion_jobs
    ADD COLUMN mode text NOT NULL DEFAULT 'soft',
    ADD COLUMN permanent_requested_by bigint,
    ADD COLUMN permanent_requested_at timestamptz;

UPDATE public.recording_deletion_jobs
   SET mode = 'permanent', permanent_requested_by = requested_by,
       permanent_requested_at = created_at;

ALTER TABLE public.recording_deletion_jobs
    DROP CONSTRAINT recording_deletion_jobs_state_check;
ALTER TABLE public.recording_deletion_jobs
    ADD CONSTRAINT recording_deletion_jobs_state_check
    CHECK (state IN ('queued', 'running', 'ready', 'failed', 'soft_deleted', 'paused'));
ALTER TABLE public.recording_deletion_jobs
    ADD CONSTRAINT recording_deletion_jobs_mode_check
    CHECK (mode IN ('soft', 'permanent'));
ALTER TABLE public.recording_deletion_jobs
    ADD CONSTRAINT recording_deletion_jobs_soft_terminal_check
    CHECK (mode = 'permanent' OR state = 'soft_deleted');

-- A previous worker might already have removed some media. Preserve that
-- uncertainty in the audit record for operator review instead of claiming
-- those sessions are safely soft-deleted or resuming their purge on restart.
UPDATE public.recording_deletion_jobs
   SET state = 'paused', stage = 'paused', attempt_token = NULL,
       lease_expires_at = NULL, retry_at = now(), updated_at = now(),
       error = 'Permanent deletion paused by soft-delete rollout; review before resuming'
 WHERE state IN ('queued', 'running', 'failed');

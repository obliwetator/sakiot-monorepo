-- Realtime job progress: media jobs, clip exports and recording deletions
-- notify `sakiot_realtime` when what their status endpoints show changes, so
-- dashboards stop polling them every second.
--
-- Payload (see 20261003010000_realtime_notifications.sql for the others):
--   job {"v":1,"k":"job","r":"media"|"composition"|"deletion","g":guild,
--        "id":job,"u":owner}
-- `u` is set for clip exports only, whose status only their owner may read.
-- `web-server` sends media job events to the job's viewers
-- (`media_job_viewers`) and deletion events to guild managers.
--
-- Only state, stage, progress and error count: lease renewals and other
-- bookkeeping send nothing. Inserts send nothing either, because whoever
-- creates or joins a job reads its status in the same request.

SET lock_timeout = '5s';

CREATE FUNCTION public.realtime_jobs() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_json jsonb := to_jsonb(NEW);
BEGIN
    -- A media job without a guild has no subscribed audience.
    IF row_json ->> 'guild_id' IS NOT NULL THEN
        PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
            'v', 1, 'k', 'job',
            'r', TG_ARGV[0],
            'g', row_json ->> 'guild_id',
            'id', row_json ->> 'id',
            'u', CASE WHEN TG_ARGV[0] = 'composition' THEN row_json ->> 'user_id' END)));
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.media_jobs
    FOR EACH ROW
    WHEN ((OLD.state, OLD.stage, OLD.progress, OLD.error)
          IS DISTINCT FROM (NEW.state, NEW.stage, NEW.progress, NEW.error))
    EXECUTE FUNCTION public.realtime_jobs('media');

CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.composition_jobs
    FOR EACH ROW
    WHEN ((OLD.state, OLD.stage, OLD.progress, OLD.error)
          IS DISTINCT FROM (NEW.state, NEW.stage, NEW.progress, NEW.error))
    EXECUTE FUNCTION public.realtime_jobs('composition');

CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.recording_deletion_jobs
    FOR EACH ROW
    WHEN ((OLD.state, OLD.stage, OLD.mode, OLD.error)
          IS DISTINCT FROM (NEW.state, NEW.stage, NEW.mode, NEW.error))
    EXECUTE FUNCTION public.realtime_jobs('deletion');

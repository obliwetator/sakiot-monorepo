-- Retention moved to `guild_limits.retention_days` in 20261106000000, where
-- the bot's operators set it, and nothing has read the per-guild column since
-- v1.13.0. Its realtime trigger names it, so the trigger is recreated without
-- it. The new one also fires when `notice_delivery` changes: 20261109000000
-- added that column without it, so changing where notices go left other open
-- Voice Settings pages showing the old choice.

SET lock_timeout = '5s';

DROP TRIGGER realtime_update ON public.guild_recording_policy;

ALTER TABLE public.guild_recording_policy DROP COLUMN retention_days;

CREATE TRIGGER realtime_update
    AFTER UPDATE ON public.guild_recording_policy
    FOR EACH ROW
    WHEN ((OLD.excluded_channel_ids, OLD.notice_delivery)
          IS DISTINCT FROM (NEW.excluded_channel_ids, NEW.notice_delivery))
    EXECUTE FUNCTION public.realtime_settings();

RESET lock_timeout;

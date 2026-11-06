-- How long a guild's recordings stay moves from its managers to the bot's
-- operators, a limit beside the kept-recordings cap. NULL here means the
-- default, which the web server holds.
--
-- A guild's current retention carries over. One with retention off has nothing
-- to carry, so it takes the default. `guild_recording_policy.retention_days`
-- stays for the release this migration runs under, which still reads it; a
-- later migration drops it.
SET lock_timeout = '5s';

ALTER TABLE public.guild_limits
    ADD COLUMN retention_days integer CHECK (retention_days BETWEEN 1 AND 3650),
    ALTER COLUMN kept_recordings SET DEFAULT 0;

INSERT INTO public.guild_limits (guild_id, retention_days, updated_by)
SELECT guild_id, retention_days, COALESCE(updated_by, 0)
  FROM public.guild_recording_policy
 WHERE retention_days IS NOT NULL
ON CONFLICT (guild_id) DO UPDATE SET retention_days = EXCLUDED.retention_days;

RESET lock_timeout;

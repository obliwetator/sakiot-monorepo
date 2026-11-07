-- What others did to the bot in voice, and how it cleaned up after them.
--
-- A deafened bot hears nothing, so a recording goes silent with no sign of
-- why. The bot writes one row when someone else deafens, undeafens, moves or
-- disconnects it, and one when it can't clear a deafen left over from an
-- earlier visit. Its own joins, switches and leaves are not logged, but clearing
-- that deafen is, as an undeafen by the bot: the timeline needs the span's end.
--
-- `actor_id` comes from Discord's audit log and `attribution` says how far to
-- trust it:
--   exact    the entry names the bot as its target
--   likely   an entry of the right kind and age that names no target, which
--            is how Discord records moves and disconnects
--   unknown  no matching entry
--   no_access  the bot may not read the audit log
-- A failed cleanup names the bot, as `exact`.

SET lock_timeout = '5s';

CREATE TABLE public.bot_voice_log (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    guild_id bigint NOT NULL,
    kind text NOT NULL CHECK (kind IN (
        'deafened', 'undeafened', 'moved', 'disconnected',
        'deafen_clear_failed'
    )),
    channel_id bigint NOT NULL,
    -- Where a move put the bot; null for every other kind.
    to_channel_id bigint,
    actor_id bigint,
    attribution text NOT NULL CHECK (attribution IN (
        'exact', 'likely', 'unknown', 'no_access'
    )),
    occurred_at timestamp with time zone NOT NULL,
    CHECK ((kind = 'moved') = (to_channel_id IS NOT NULL)),
    CHECK ((attribution IN ('exact', 'likely')) = (actor_id IS NOT NULL))
);

CREATE INDEX bot_voice_log_guild_time
    ON public.bot_voice_log (guild_id, occurred_at DESC, id DESC);

RESET lock_timeout;

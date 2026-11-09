-- Telling members they are recorded.
--
-- Recording is on by default, so the bot tells each member once per guild. The
-- row is queued in the transaction that opens the member's first recording,
-- which means nobody is recorded without one, and a worker delivers it. It
-- stays afterwards as the record of what was sent, when and how.
--
-- `status`:
--   pending  waiting for delivery; `next_attempt_at` is also the claim's lease
--   sent     delivered; `delivered_via` and `message_id` say where
--   skipped  nobody to tell: a bot, or a member who had already opted out
--   failed   every route refused it, or retries ran out. Queued again if the
--            member is still being recorded a day later
--
-- `channel_id` is the voice channel of that first recording: a channel notice
-- goes to its text chat.

SET lock_timeout = '5s';

ALTER TABLE public.guild_recording_policy
    ADD COLUMN notice_delivery text NOT NULL DEFAULT 'dm'
        CHECK (notice_delivery IN ('dm', 'channel'));

CREATE TABLE public.recording_notices (
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    channel_id bigint NOT NULL,
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sent', 'skipped', 'failed')),
    queued_at timestamp with time zone NOT NULL DEFAULT now(),
    next_attempt_at timestamp with time zone NOT NULL DEFAULT now(),
    attempts integer NOT NULL DEFAULT 0,
    sent_at timestamp with time zone,
    delivered_via text CHECK (delivered_via IN ('dm', 'channel')),
    message_id bigint,
    last_error text,
    PRIMARY KEY (guild_id, user_id),
    CHECK ((status = 'sent') = (sent_at IS NOT NULL)),
    CHECK ((status = 'sent') = (delivered_via IS NOT NULL)),
    CHECK ((status = 'sent') = (message_id IS NOT NULL))
);

CREATE INDEX recording_notices_due
    ON public.recording_notices (next_attempt_at)
    WHERE status = 'pending';

RESET lock_timeout;

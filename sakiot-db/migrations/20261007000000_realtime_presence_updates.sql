-- Voice presence notifications name the member and the channel they were in,
-- so the server can push each change to the subscribers allowed to see it
-- instead of every subscriber refetching the guild's whole presence list.
--
--   presence {"v":1,"k":"presence","g":guild,"u":user,"o":old channel}
--
-- `o` is the channel the member was in before the change, absent when they
-- joined. Where they are now is read when the change is routed. Payloads
-- without `u` (a rename of someone in voice, an ownership change) still mean
-- "refetch the whole list".

SET lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.realtime_voice_presence() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    payload jsonb;
BEGIN
    IF TG_OP = 'INSERT' THEN
        payload := jsonb_build_object(
            'v', 1, 'k', 'presence', 'g', NEW.guild_id::text, 'u', NEW.user_id::text);
    ELSE
        payload := jsonb_build_object(
            'v', 1, 'k', 'presence', 'g', OLD.guild_id::text, 'u', OLD.user_id::text,
            'o', OLD.channel_id::text);
    END IF;
    PERFORM public.realtime_notify(payload);
    RETURN NULL;
END
$$;

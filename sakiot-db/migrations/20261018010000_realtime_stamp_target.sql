-- A stamp is on its target's voice, which is always theirs to hear, so its
-- payload names the target:
--   stamp {"v":1,"k":"stamp","g":guild,"id":stamp,"c":channel,"s":session,"u":target}

CREATE OR REPLACE FUNCTION public.realtime_stamps() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.stamps;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_strip_nulls(jsonb_build_object(
        'v', 1, 'k', 'stamp',
        'g', row_data.guild_id::text,
        'id', row_data.id::text,
        'c', row_data.channel_id::text,
        's', row_data.recording_session_id::text,
        'u', row_data.target_user_id::text)));
    RETURN NULL;
END
$$;

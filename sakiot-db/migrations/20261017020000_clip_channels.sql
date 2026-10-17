-- A clip is heard by whoever may listen in every channel its own audio came
-- from. Until now a clip cut from a session needed every channel the whole
-- session went through, so a clip of the part recorded in one channel was
-- hidden from members of that channel who could not hear the others.
--
-- `clips.channel_ids` records those channels when the clip is made:
--   * cut from one recording: that recording's channel;
--   * cut from a session's timeline: the channels of the recordings its
--     range covers, or every channel of the session when it covers none
--     (silence only);
--   * cut from a session's silence-free copy, whose times no longer match
--     the recordings: every channel of the session;
--   * composed: every channel of its sources.
-- A clip's audio never changes, except a composed clip being overwritten,
-- which rewrites its channels too. `channel_id` stays: the dashboard shows
-- it, and recording deletion matches clips of single recordings by it.
--
-- Clip payloads carry the channels instead of the channel and session:
--   clip {"v":1,"k":"clip","g":guild,"id":clip,"cs":[channel, ...]}
-- A change of channels also sends the old ones, so whoever could hear the
-- clip before hears that it changed.

SET lock_timeout = '5s';

ALTER TABLE public.clips ADD COLUMN channel_ids bigint[];

-- The backfill writes every clip; nobody needs a notification for each.
ALTER TABLE public.clips DISABLE TRIGGER realtime_update;

-- Clips of one recording, and composed clips until their sources are known.
UPDATE public.clips
   SET channel_ids = ARRAY[channel_id]
 WHERE recording_session_id IS NULL
   AND channel_id IS NOT NULL;

-- Clips of a session. The range is widened by a second on each side, so
-- `real` rounding of the stored offsets can only add a neighbouring
-- recording's channel, never miss one.
UPDATE public.clips c
   SET channel_ids = COALESCE(
           (SELECT array_agg(DISTINCT af.channel_id ORDER BY af.channel_id)
              FROM public.audio_files af
             WHERE af.recording_session_id = c.recording_session_id
               AND af.channel_id IS NOT NULL
               AND NOT c.silence_free
               AND c.length IS NOT NULL
               AND COALESCE(af.start_ts, 0)
                   < rs.started_ms + ((c.start_time + c.length)::float8 * 1000)::bigint + 1000
               AND (af.end_ts IS NULL
                    OR af.end_ts > rs.started_ms + (c.start_time::float8 * 1000)::bigint - 1000)),
           (SELECT array_agg(DISTINCT af.channel_id ORDER BY af.channel_id)
              FROM public.audio_files af
             WHERE af.recording_session_id = c.recording_session_id
               AND af.channel_id IS NOT NULL),
           ARRAY[rs.starting_channel_id])
  FROM (SELECT id,
               starting_channel_id,
               (EXTRACT(EPOCH FROM started_at) * 1000)::bigint AS started_ms
          FROM public.recording_sessions) rs
 WHERE rs.id = c.recording_session_id;

-- Composed clips: every channel of their sources. A source that no longer
-- exists counts as the composed clip's own channel, the only one it was
-- checked against before.
WITH segments AS (
    SELECT composed.clip_id,
           composed.guild_id,
           composed.channel_id,
           segment ->> 'source_id' AS source_id
      FROM public.clips composed
     CROSS JOIN LATERAL jsonb_array_elements(
               CASE WHEN jsonb_typeof(composed.composition -> 'segments') = 'array'
                    THEN composed.composition -> 'segments'
                    ELSE '[]'::jsonb
               END) segment
     WHERE composed.composition IS NOT NULL
), sources AS (
    SELECT segments.clip_id,
           array_agg(DISTINCT channel ORDER BY channel) AS channel_ids
      FROM segments
      LEFT JOIN public.clips source
             ON source.clip_id = segments.source_id
            AND source.guild_id = segments.guild_id
     CROSS JOIN LATERAL unnest(
               CASE WHEN source.clip_id IS NULL THEN ARRAY[segments.channel_id]
                    ELSE source.channel_ids
               END) channel
     WHERE channel IS NOT NULL
     GROUP BY segments.clip_id
)
UPDATE public.clips c
   SET channel_ids = sources.channel_ids
  FROM sources
 WHERE c.clip_id = sources.clip_id;

-- Neither listed nor playable before (no channel, or a session that is
-- gone): no one may hear them.
UPDATE public.clips SET channel_ids = '{}' WHERE channel_ids IS NULL;

ALTER TABLE public.clips ALTER COLUMN channel_ids SET NOT NULL;

ALTER TABLE public.clips ENABLE TRIGGER realtime_update;

COMMENT ON COLUMN public.clips.channel_ids IS
    'The channels this clip''s audio came from; hearing the clip takes listening access in every one. Empty: no one may hear it.';

CREATE FUNCTION public.realtime_clip(clip public.clips) RETURNS void
    LANGUAGE plpgsql
AS $$
BEGIN
    -- Listings only ever show clips of one guild; a guild-less clip has no
    -- audience.
    IF clip.guild_id IS NOT NULL THEN
        PERFORM public.realtime_notify(jsonb_build_object(
            'v', 1, 'k', 'clip',
            'g', clip.guild_id::text,
            'id', clip.clip_id,
            'cs', to_jsonb(clip.channel_ids::text[])));
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION public.realtime_clips() RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM public.realtime_clip(OLD);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' AND OLD.channel_ids IS DISTINCT FROM NEW.channel_ids THEN
        PERFORM public.realtime_clip(OLD);
    END IF;
    PERFORM public.realtime_clip(NEW);
    RETURN NULL;
END
$$;

RESET lock_timeout;

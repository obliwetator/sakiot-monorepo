-- Whose voices a clip holds. A member may always hear a clip of only their
-- own voice, wherever it was recorded; anyone else's voice takes listening
-- access in every channel it came from (`channel_ids`). Later, a speaker's
-- listening consent will apply to every clip holding their voice.
--
-- `clips.speaker_ids` records them when the clip is made:
--   * cut from one recording: that recording's speaker;
--   * cut from a session: the session's speaker (sessions are per user);
--   * composed: every speaker of its sources.
-- Empty means not known, which never counts as anyone's own clip.
--
-- Clip payloads carry the speakers too:
--   clip {"v":1,"k":"clip","g":guild,"id":clip,"cs":[channel, ...],"ss":[speaker, ...]}
-- A change of speakers, like a change of channels, also sends the old ones.

SET lock_timeout = '5s';

ALTER TABLE public.clips ADD COLUMN speaker_ids bigint[];

-- The backfill writes every clip; nobody needs a notification for each.
ALTER TABLE public.clips DISABLE TRIGGER realtime_update;

-- Clips of a session.
UPDATE public.clips c
   SET speaker_ids = ARRAY[rs.user_id]
  FROM public.recording_sessions rs
 WHERE rs.id = c.recording_session_id;

-- Clips of one recording: the speaker of the recording the clip names, or,
-- once that row is gone, the speaker its name carries. The bot names every
-- recording `<start ms>-<speaker id>`, and older ones `-<username>` after
-- that.
UPDATE public.clips c
   SET speaker_ids = ARRAY[COALESCE(
           (SELECT af.user_id
              FROM public.audio_files af
             WHERE af.guild_id = c.guild_id
               AND af.channel_id = c.channel_id
               AND af.file_name IN (c.original_file_name, c.original_file_name || '.ogg')
             ORDER BY af.id DESC
             LIMIT 1),
           (SELECT named::bigint
              FROM substring(c.original_file_name
                             FROM '^[0-9]+-([0-9]{1,19})(?:-|\.ogg$|$)') named
             WHERE named::numeric <= 9223372036854775807))]
 WHERE c.recording_session_id IS NULL
   AND c.composition IS NULL
   AND c.original_file_name IS DISTINCT FROM 'compose';

-- Composed clips: every speaker of their sources, once every source's
-- speakers are known. One unknown source leaves the whole clip unknown:
-- the rest alone could make another speaker's voice look like your own.
WITH segments AS (
    SELECT composed.clip_id,
           composed.guild_id,
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
           bool_and(source.speaker_ids IS NOT NULL
                    AND cardinality(source.speaker_ids) > 0
                    AND array_position(source.speaker_ids, NULL) IS NULL) AS known,
           array_agg(DISTINCT speaker ORDER BY speaker)
               FILTER (WHERE speaker IS NOT NULL) AS speaker_ids
      FROM segments
      LEFT JOIN public.clips source
             ON source.clip_id = segments.source_id
            AND source.guild_id = segments.guild_id
      LEFT JOIN LATERAL unnest(source.speaker_ids) speaker ON true
     GROUP BY segments.clip_id
)
UPDATE public.clips c
   SET speaker_ids = sources.speaker_ids
  FROM sources
 WHERE c.clip_id = sources.clip_id
   AND sources.known;

-- Speakers not known: no name to read, or a source that is gone.
UPDATE public.clips
   SET speaker_ids = '{}'
 WHERE speaker_ids IS NULL
    OR array_position(speaker_ids, NULL) IS NOT NULL;

ALTER TABLE public.clips ALTER COLUMN speaker_ids SET NOT NULL;

ALTER TABLE public.clips ENABLE TRIGGER realtime_update;

COMMENT ON COLUMN public.clips.speaker_ids IS
    'Whose voices this clip holds. A clip of only your own voice is always yours to hear; empty: not known.';

CREATE OR REPLACE FUNCTION public.realtime_clip(clip public.clips) RETURNS void
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
            'cs', to_jsonb(clip.channel_ids::text[]),
            'ss', to_jsonb(clip.speaker_ids::text[])));
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
    IF TG_OP = 'UPDATE'
       AND (OLD.channel_ids IS DISTINCT FROM NEW.channel_ids
            OR OLD.speaker_ids IS DISTINCT FROM NEW.speaker_ids) THEN
        PERFORM public.realtime_clip(OLD);
    END IF;
    PERFORM public.realtime_clip(NEW);
    RETURN NULL;
END
$$;

RESET lock_timeout;

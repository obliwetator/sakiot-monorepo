-- Composed clips carry the serialized clip editor edit (source clips, trims,
-- timeline positions, and effects) that produced them. This is what makes a
-- composed clip re-importable into the editor for further adjustments and
-- re-exports. NULL for clips cut from a single recording.

ALTER TABLE public.clips
    ADD COLUMN IF NOT EXISTS composition jsonb;

COMMENT ON COLUMN public.clips.composition IS
    'Serialized clip editor edit (sources, trims, effects) that produced this composed clip; NULL for clips cut from a single recording.';

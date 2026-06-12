-- Discord channel types are an externally defined, extensible integer enum.
-- A lookup-table FK makes guild cache sync fail whenever Discord adds a type
-- that this database has not seeded yet.
ALTER TABLE public.channels
    DROP CONSTRAINT IF EXISTS "FK_channels_channel_type";

COMMENT ON COLUMN public.channels.type IS
    'Discord channel type integer; unknown future values are preserved.';

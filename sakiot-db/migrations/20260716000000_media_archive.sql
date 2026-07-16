-- Durable archive state for finalized recording originals and saved clips.
-- Object keys are populated only after workers hash local bytes; no Discord
-- guild/channel/user identifiers are copied into remote object names.

CREATE TABLE public.media_objects (
    id bigserial PRIMARY KEY,
    audio_file_id bigint,
    clip_id character varying(255),
    state text NOT NULL DEFAULT 'pending',
    object_key text,
    bytes bigint,
    sha256 character(64),
    etag text,
    attempts integer NOT NULL DEFAULT 0,
    retry_at timestamp with time zone NOT NULL DEFAULT now(),
    lease_owner text,
    lease_expires_at timestamp with time zone,
    last_error text,
    uploaded_at timestamp with time zone,
    verified_at timestamp with time zone,
    local_delete_after timestamp with time zone,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    updated_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT media_objects_exactly_one_source_check
        CHECK (num_nonnulls(audio_file_id, clip_id) = 1),
    CONSTRAINT media_objects_state_check
        CHECK (state = ANY (ARRAY['pending', 'uploading', 'available', 'missing', 'conflict']::text[])),
    CONSTRAINT media_objects_bytes_check
        CHECK (bytes IS NULL OR bytes >= 0),
    CONSTRAINT media_objects_attempts_check
        CHECK (attempts >= 0),
    CONSTRAINT media_objects_sha256_check
        CHECK (sha256 IS NULL OR sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT media_objects_available_metadata_check
        CHECK (
            state <> 'available'
            OR (
                object_key IS NOT NULL
                AND bytes IS NOT NULL
                AND sha256 IS NOT NULL
                AND uploaded_at IS NOT NULL
                AND verified_at IS NOT NULL
                AND local_delete_after IS NOT NULL
            )
        ),
    CONSTRAINT media_objects_audio_file_id_fkey
        FOREIGN KEY (audio_file_id)
        REFERENCES public.audio_files(id)
        ON DELETE RESTRICT,
    CONSTRAINT media_objects_clip_id_fkey
        FOREIGN KEY (clip_id)
        REFERENCES public.clips(clip_id)
        ON DELETE RESTRICT
);

CREATE UNIQUE INDEX media_objects_audio_file_unique_idx
    ON public.media_objects (audio_file_id)
    WHERE audio_file_id IS NOT NULL;

CREATE UNIQUE INDEX media_objects_clip_unique_idx
    ON public.media_objects (clip_id)
    WHERE clip_id IS NOT NULL;

CREATE UNIQUE INDEX media_objects_object_key_unique_idx
    ON public.media_objects (object_key)
    WHERE object_key IS NOT NULL;

CREATE INDEX media_objects_claim_idx
    ON public.media_objects (retry_at, created_at, id)
    WHERE state IN ('pending', 'uploading', 'missing');

CREATE INDEX media_objects_lease_idx
    ON public.media_objects (lease_expires_at)
    WHERE lease_owner IS NOT NULL;

CREATE INDEX media_objects_local_delete_idx
    ON public.media_objects (local_delete_after, id)
    WHERE state = 'available' AND verified_at IS NOT NULL;

INSERT INTO public.media_objects (audio_file_id)
SELECT af.id
  FROM public.audio_files af
 WHERE af.end_ts IS NOT NULL
ON CONFLICT (audio_file_id) WHERE audio_file_id IS NOT NULL DO NOTHING;

INSERT INTO public.media_objects (clip_id)
SELECT c.clip_id
  FROM public.clips c
 WHERE c.saved_file_name IS NOT NULL
   AND btrim(c.saved_file_name) <> ''
ON CONFLICT (clip_id) WHERE clip_id IS NOT NULL DO NOTHING;

COMMENT ON TABLE public.media_objects IS
    'Durable B2 archive queue and verification ledger. Runtime credentials intentionally have no object-delete capability.';

COMMENT ON COLUMN public.media_objects.local_delete_after IS
    'Earliest normal local eviction time, set only after full remote SHA-256 verification and reset after remote hydration.';

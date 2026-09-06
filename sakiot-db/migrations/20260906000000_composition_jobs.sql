-- Accepted exports outlive HTTP requests and service releases. saved_file_name
-- is an immutable media revision; overwrite publication compares it atomically.
CREATE TABLE composition_jobs (
    id text PRIMARY KEY,
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    idempotency_key text NOT NULL,
    request jsonb NOT NULL,
    snapshot jsonb NOT NULL,
    renderer_version integer NOT NULL DEFAULT 1,
    result_clip_id text NOT NULL,
    state text NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'ready', 'failed')),
    stage text NOT NULL DEFAULT 'queued',
    progress smallint NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    attempt_token text,
    lease_expires_at timestamptz,
    retry_at timestamptz NOT NULL DEFAULT now(),
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    UNIQUE (guild_id, user_id, idempotency_key),
    CHECK ((state = 'running') = (attempt_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);
CREATE INDEX composition_jobs_queue_idx ON composition_jobs (retry_at, created_at)
    WHERE state IN ('queued', 'running');
CREATE INDEX composition_jobs_owner_idx ON composition_jobs (guild_id, user_id, created_at DESC);

-- Bind archive work and eviction to the bytes they were scheduled against.
ALTER TABLE media_objects ADD COLUMN clip_saved_file_name text;
UPDATE media_objects m SET clip_saved_file_name = c.saved_file_name
    FROM clips c WHERE c.clip_id = m.clip_id;

CREATE FUNCTION capture_clip_archive_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.clip_id IS NOT NULL AND NEW.clip_saved_file_name IS NULL THEN
        SELECT saved_file_name INTO NEW.clip_saved_file_name FROM clips WHERE clip_id = NEW.clip_id;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER media_objects_capture_clip_revision BEFORE INSERT ON media_objects
    FOR EACH ROW EXECUTE FUNCTION capture_clip_archive_revision();

-- Older web releases can overlap this additive migration. Keep their clip
-- overwrites revision-safe too, even before the new publisher is running.
CREATE FUNCTION invalidate_clip_archive_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.saved_file_name IS DISTINCT FROM OLD.saved_file_name THEN
        UPDATE media_objects SET clip_saved_file_name = NEW.saved_file_name,
            state = 'pending', retry_at = now(), lease_owner = NULL,
            lease_expires_at = NULL, object_key = NULL, bytes = NULL, sha256 = NULL,
            etag = NULL, attempts = 0, last_error = NULL, uploaded_at = NULL,
            verified_at = NULL, local_delete_after = NULL, updated_at = now()
        WHERE clip_id = NEW.clip_id;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER clips_invalidate_archive_revision AFTER UPDATE OF saved_file_name ON clips
    FOR EACH ROW EXECUTE FUNCTION invalidate_clip_archive_revision();

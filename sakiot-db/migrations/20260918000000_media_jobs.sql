-- Durable, shared queue for expensive media work. Composition jobs keep their
-- richer editor-specific snapshot, but both queues participate in the same
-- admission and running limits in application code.
CREATE TABLE media_jobs (
    id text PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN (
        'recording_silence',
        'recording_waveform',
        'clip_waveform',
        'session_waveform',
        'session_silence',
        'session_mix',
        'session_download'
    )),
    guild_id bigint,
    user_id bigint NOT NULL,
    idempotency_key text NOT NULL,
    resource_key text NOT NULL,
    request jsonb NOT NULL,
    state text NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'ready', 'failed')),
    stage text NOT NULL DEFAULT 'queued',
    progress smallint NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    attempt_token text,
    lease_expires_at timestamptz,
    retry_at timestamptz NOT NULL DEFAULT now(),
    result_url text,
    result_path text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    UNIQUE (user_id, kind, idempotency_key),
    CHECK ((state = 'running') = (attempt_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);

CREATE INDEX media_jobs_queue_idx ON media_jobs (retry_at, created_at)
    WHERE state IN ('queued', 'running');
CREATE INDEX media_jobs_owner_idx ON media_jobs (user_id, created_at DESC);
CREATE UNIQUE INDEX media_jobs_active_resource_idx ON media_jobs (kind, resource_key)
    WHERE state IN ('queued', 'running');

-- Authorized callers may attach to one shared resource build without learning
-- about unrelated jobs or duplicating expensive work.
CREATE TABLE media_job_viewers (
    job_id text NOT NULL REFERENCES media_jobs(id) ON DELETE CASCADE,
    user_id bigint NOT NULL,
    PRIMARY KEY (job_id, user_id)
);
CREATE INDEX media_job_viewers_user_idx ON media_job_viewers (user_id, job_id);

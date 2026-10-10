-- One row per sign-in, Discord or dev login. Both tokens carry the row's id,
-- and /api/refresh succeeds only while the row exists, so deleting it signs
-- that browser out once its 15-minute access token expires. Logout deletes
-- its own row; "sign out everywhere" deletes every row of the user and login
-- kind. A refresh moves refreshed_at, and a session nobody refreshed for the
-- refresh token's lifetime can never refresh again, so sign-in sweeps it.
CREATE TABLE public.auth_sessions (
    id text PRIMARY KEY,
    user_id bigint NOT NULL,
    auth_kind text NOT NULL CHECK (auth_kind IN ('discord', 'dev')),
    created_at timestamptz NOT NULL DEFAULT now(),
    refreshed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX auth_sessions_user_idx ON public.auth_sessions (user_id, auth_kind);
CREATE INDEX auth_sessions_refreshed_at_idx ON public.auth_sessions (refreshed_at);

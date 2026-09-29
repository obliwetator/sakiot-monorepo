-- Members who asked the bot not to record their voice in a guild. Recording is
-- on by default; a row here stops new recording fragments for that user and
-- makes the bot close any that are open. Earlier recordings are kept.
CREATE TABLE public.recording_opt_outs (
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, user_id)
);

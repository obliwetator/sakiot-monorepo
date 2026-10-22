-- Sakiot's own permission rules, layered over Discord's (`sakiot-permissions`,
-- `Rules`). A rule allows or denies Sakiot capabilities, as bits of the
-- crate's `Capabilities` (never Discord's), to `@everyone`, a role or a user,
-- guild-wide (`channel_id` NULL) or in one voice or stage channel. At most one
-- rule per scope and subject; an empty one is no row.
--
-- No foreign keys to `roles` or `channels`: those are the bot's projections,
-- and a projection that prunes a guild's roles must not take user-written
-- rules with it. A rule whose role or channel is gone reaches nobody, and the
-- editor offers to delete it.
--
-- Realtime: a change re-authorizes the guild's viewers like any permission
-- change (`perm`), and refreshes the rule editor:
--   settings {"v":1,"k":"settings","g":guild,"r":"permission_rules"}

SET lock_timeout = '5s';

CREATE TABLE public.permission_rules (
    guild_id bigint NOT NULL,
    channel_id bigint,
    subject_kind text NOT NULL,
    subject_id bigint,
    allow bigint NOT NULL,
    deny bigint NOT NULL,
    updated_by bigint NOT NULL,
    updated_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT permission_rules_subject CHECK (
        subject_kind IN ('everyone', 'role', 'user')
        AND (subject_kind = 'everyone') = (subject_id IS NULL)),
    CONSTRAINT permission_rules_effect CHECK (allow & deny = 0 AND (allow | deny) <> 0),
    CONSTRAINT permission_rules_one_per_subject
        UNIQUE NULLS NOT DISTINCT (guild_id, channel_id, subject_kind, subject_id)
);

COMMENT ON TABLE public.permission_rules IS
    'Sakiot''s own permission rules over Discord''s: capability bits allowed or denied to @everyone, a role or a user, guild-wide (channel_id NULL) or in one channel.';

CREATE FUNCTION public.realtime_permission_rules() RETURNS trigger
    LANGUAGE plpgsql
AS $$
DECLARE
    row_data public.permission_rules;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := OLD;
    ELSE
        row_data := NEW;
    END IF;
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'perm', 'g', row_data.guild_id::text));
    PERFORM public.realtime_notify(jsonb_build_object(
        'v', 1, 'k', 'settings',
        'g', row_data.guild_id::text,
        'r', 'permission_rules'));
    RETURN NULL;
END
$$;

CREATE TRIGGER realtime_change
    AFTER INSERT OR UPDATE OR DELETE ON public.permission_rules
    FOR EACH ROW EXECUTE FUNCTION public.realtime_permission_rules();

RESET lock_timeout;

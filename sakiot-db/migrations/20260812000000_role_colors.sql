-- Discord roles carry a color (0 = unset) and, for guilds with the
-- ENHANCED_ROLE_COLORS feature, optional secondary/tertiary colors that turn
-- the role into a gradient. The web UI renders these alongside role names in
-- the members page. color_secondary/color_tertiary are NULL when the role has
-- no gradient.

ALTER TABLE public.roles
    ADD COLUMN IF NOT EXISTS color bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS color_secondary bigint,
    ADD COLUMN IF NOT EXISTS color_tertiary bigint;

COMMENT ON COLUMN public.roles.color IS
    'Primary Discord role color as a 24-bit RGB integer; 0 means unset.';
COMMENT ON COLUMN public.roles.color_secondary IS
    'Second color of a role gradient; NULL when the role has no gradient.';
COMMENT ON COLUMN public.roles.color_tertiary IS
    'Third color of a holographic role gradient; NULL when the role has none.';

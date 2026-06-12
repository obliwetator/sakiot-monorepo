-- Minimal local-dev seed. Idempotent: every insert is ON CONFLICT DO NOTHING.
-- Run via scripts/dev.sh, which passes -v dev_id=<DEV_ACCOUNT_ID>.

INSERT INTO discord_auth_user (id, username, discriminator, avatar, email, flags, public_flags)
VALUES (:dev_id, 'local-dev', '0', '', 'dev@localhost', 0, 0)
ON CONFLICT DO NOTHING;

INSERT INTO guilds (id, owner_id)
VALUES (111111111111111111, :dev_id)
ON CONFLICT DO NOTHING;

INSERT INTO guilds_present (guild_id)
VALUES (111111111111111111)
ON CONFLICT DO NOTHING;

-- owner = true grants Permissions::all() (web_server/src/permissions.rs)
INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
VALUES (111111111111111111, :dev_id, 'Local Dev Guild', NULL, true, 8, '{}')
ON CONFLICT DO NOTHING;

-- @everyone role (role_id = guild_id); permission lookups fetch_one this row
INSERT INTO roles (guild_id, role_id, permission, name)
VALUES (111111111111111111, 111111111111111111, 1049600, '@everyone')
ON CONFLICT DO NOTHING;

INSERT INTO channels (channel_id, guild_id, type, name)
VALUES
    (111111111111111112, 111111111111111111, 2, 'General Voice'),
    (111111111111111113, 111111111111111111, 0, 'general')
ON CONFLICT DO NOTHING;

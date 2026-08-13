-- Minimal local-dev seed. Idempotent: every insert is ON CONFLICT DO NOTHING.
-- Run via `cargo dev db up`, which substitutes <DEV_ACCOUNT_ID> atomically.

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

-- Sample roles so the members page has something to filter by out of the box.
-- 8 = ADMINISTRATOR, 32 = MANAGE_GUILD, 1024 = VIEW_CHANNEL.
-- VIP gets a gradient (color + color_secondary) so role colors are visible.
INSERT INTO roles (guild_id, role_id, permission, name, color, color_secondary)
VALUES
    (111111111111111111, 111111111111111112, 40, 'Moderator', 16711680, NULL),
    (111111111111111111, 111111111111111113, 1024, 'VIP', 3050327, 16743936)
ON CONFLICT DO NOTHING;

-- Sample members with names (user_names) and, for one, a guild nickname.
INSERT INTO user_names (user_id, username, global_name)
VALUES
    (100000000000000001, 'alice', NULL),
    (100000000000000002, 'bob', 'Bobby'),
    (100000000000000003, 'carol', NULL),
    (100000000000000004, 'dave', NULL)
ON CONFLICT DO NOTHING;

INSERT INTO user_nicknames (user_id, guild_id, nickname)
VALUES (100000000000000002, 111111111111111111, 'BobbyNick')
ON CONFLICT DO NOTHING;

-- alice and carol moderate; bob and carol are VIPs; dave has no role.
INSERT INTO user_roles (user_id, role_id)
VALUES
    (100000000000000001, 111111111111111112),
    (100000000000000003, 111111111111111112),
    (100000000000000002, 111111111111111113),
    (100000000000000003, 111111111111111113)
ON CONFLICT DO NOTHING;

INSERT INTO channels (channel_id, guild_id, type, name)
VALUES
    (111111111111111112, 111111111111111111, 2, 'General Voice'),
    (111111111111111113, 111111111111111111, 0, 'general')
ON CONFLICT DO NOTHING;

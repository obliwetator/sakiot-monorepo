-- Read-only check before realtime phase 4 ships: who the switch from
-- `user_guilds` (the guild list Discord returned at login) to the bot's
-- roster (`guild_members`) would change access for. Every query should
-- return no rows; anything listed loses access, or owner rights, on deploy.
--
--   psql -d <db> -f membership-parity.sql
--
-- Dev logins keep the `user_guilds` rules, so only Discord logins matter here
-- (dev logins do not exist in production builds).

\echo 'Bot-present guilds without a complete roster (every member would get 503 until it completes):'
SELECT gp.guild_id
  FROM guilds_present gp
  LEFT JOIN guild_projection_state s ON s.guild_id = gp.guild_id
 WHERE s.roster_complete_at IS NULL;

\echo 'Logged-in members missing from the roster (would lose access):'
SELECT ug.id AS guild_id, ug.user_id
  FROM user_guilds ug
  JOIN guilds_present gp ON gp.guild_id = ug.id
  LEFT JOIN guilds g ON g.id = ug.id
 WHERE g.owner_id IS DISTINCT FROM ug.user_id
   AND NOT EXISTS (
       SELECT 1 FROM guild_members m
        WHERE m.guild_id = ug.id AND m.user_id = ug.user_id);

\echo 'Login owner flags that guilds.owner_id does not confirm (would lose owner rights):'
SELECT ug.id AS guild_id, ug.user_id, g.owner_id AS recorded_owner
  FROM user_guilds ug
  JOIN guilds_present gp ON gp.guild_id = ug.id
  LEFT JOIN guilds g ON g.id = ug.id
 WHERE ug.owner
   AND g.owner_id IS DISTINCT FROM ug.user_id;

-- Backfill name-history baselines before the oldest known recording.
-- This lets historical listing resolve a name even for users who have never
-- changed username/global name/nickname since history collection was added.

WITH baseline AS (
    SELECT to_timestamp(MIN(start_ts) / 1000.0) - INTERVAL '1 millisecond' AS observed_at
    FROM audio_files
    WHERE start_ts IS NOT NULL
)
INSERT INTO user_name_history (user_id, guild_id, kind_id, value, observed_at)
SELECT un.user_id, NULL::BIGINT, 1, un.username, baseline.observed_at
FROM user_names un
CROSS JOIN baseline
WHERE baseline.observed_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM user_name_history h
      WHERE h.user_id = un.user_id
        AND h.guild_id IS NULL
        AND h.kind_id = 1
        AND h.observed_at <= baseline.observed_at
  );

WITH baseline AS (
    SELECT to_timestamp(MIN(start_ts) / 1000.0) - INTERVAL '1 millisecond' AS observed_at
    FROM audio_files
    WHERE start_ts IS NOT NULL
)
INSERT INTO user_name_history (user_id, guild_id, kind_id, value, observed_at)
SELECT un.user_id, NULL::BIGINT, 2, un.global_name, baseline.observed_at
FROM user_names un
CROSS JOIN baseline
WHERE baseline.observed_at IS NOT NULL
  AND un.global_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM user_name_history h
      WHERE h.user_id = un.user_id
        AND h.guild_id IS NULL
        AND h.kind_id = 2
        AND h.observed_at <= baseline.observed_at
  );

WITH baseline AS (
    SELECT to_timestamp(MIN(start_ts) / 1000.0) - INTERVAL '1 millisecond' AS observed_at
    FROM audio_files
    WHERE start_ts IS NOT NULL
)
INSERT INTO user_name_history (user_id, guild_id, kind_id, value, observed_at)
SELECT nn.user_id, nn.guild_id, 3, nn.nickname, baseline.observed_at
FROM user_nicknames nn
CROSS JOIN baseline
WHERE baseline.observed_at IS NOT NULL
  AND nn.nickname IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM user_name_history h
      WHERE h.user_id = nn.user_id
        AND h.guild_id = nn.guild_id
        AND h.kind_id = 3
        AND h.observed_at <= baseline.observed_at
  );

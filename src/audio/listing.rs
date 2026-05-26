use std::collections::{HashMap, HashSet};

use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::{get, web, HttpResponse};
use sqlx::{Pool, Postgres};
use tracing::error;

use crate::auth::{Access, Token};
use crate::errors::AppError;

use super::paths::RECORDING_PATH;
use super::types::{Channels, Directories, File};

const KIND_USERNAME: i32 = 1;
const KIND_GLOBAL_NAME: i32 = 2;
const KIND_NICKNAME: i32 = 3;

static FALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Parse `(start_ts_ms, user_id)` from a file stem like `{ts}-{uid}` or legacy
/// `{ts}-{uid}-{username}`. Strips `.ogg` if present.
fn parse_user_and_ts(file_name: &str) -> Option<(i64, i64)> {
    let stem = file_name.strip_suffix(".ogg").unwrap_or(file_name);
    let mut parts = stem.split('-');
    let ts = parts.next()?.parse::<i64>().ok()?;
    let uid = parts.next()?.parse::<i64>().ok()?;
    Some((ts, uid))
}

#[inline]
pub async fn for_entry(
    mut entries: tokio::fs::ReadDir,
    _channel: i64,
    dirs: &mut Directories,
    month_as_int: i32,
) {
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                error!("error for file: {}", err);
                continue;
            }
        };
        let file_name_str = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Skip cache directories created by the live-HLS module
        // (`hls-{stem}/`). They are not recordings.
        if file_name_str.starts_with("hls-") || file_name_str.starts_with("mix-") {
            continue;
        }
        // Only list real recording files.
        if !file_name_str.ends_with(".ogg") {
            continue;
        }
        let parsed = parse_user_and_ts(&file_name_str);
        let file_name = File {
            file: file_name_str,
            user_id: parsed.map(|(_, u)| u.to_string()),
            display_name: None,
            start_ts_ms: parsed.map(|(ts, _)| ts),
        };
        if let Some(months) = dirs.months.as_mut() {
            if let Some(Some(files)) = months.get_mut(&month_as_int) {
                files.push(file_name);
            }
        }
    }
}

/// Per-user history sorted ascending by `observed_ms`. NULL values preserved
/// — the resolver decides how to fall through them.
#[derive(Default)]
struct UserHistory {
    nickname: Vec<(i64, Option<String>)>,
    global_name: Vec<(i64, Option<String>)>,
    username: Vec<(i64, Option<String>)>,
}

/// Latest entry with `observed_ms <= start_ts_ms`. Returns None when no row
/// predates start_ts, OR when the predating row had NULL value (fall-through
/// to the next kind — matches Discord cleared-nickname UX).
fn latest_at(events: &[(i64, Option<String>)], start_ts_ms: i64) -> Option<&str> {
    let idx = events.partition_point(|(ts, _)| *ts <= start_ts_ms);
    if idx == 0 {
        return None;
    }
    events[idx - 1].1.as_deref()
}

async fn enrich_display_names(
    pool: &Pool<Postgres>,
    guild_id: i64,
    channels: &mut [Channels],
) -> Result<(), sqlx::Error> {
    let mut user_ids: HashSet<i64> = HashSet::new();
    for ch in channels.iter() {
        for dir in &ch.dirs {
            if let Some(months) = &dir.months {
                for files in months.values().flatten() {
                    for f in files {
                        if let Some(uid) = f.user_id.as_deref().and_then(|s| s.parse::<i64>().ok())
                        {
                            user_ids.insert(uid);
                        }
                    }
                }
            }
        }
    }
    if user_ids.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = user_ids.into_iter().collect();

    // Bulk history fetch. kind=3 rows are guild-scoped; kind=1/2 are always
    // NULL guild_id. Widening this filter would leak nicknames across guilds.
    let history_rows = sqlx::query!(
        r#"
        WITH ids AS (SELECT unnest($1::bigint[]) AS user_id)
        SELECT h.user_id                                          as "user_id!",
               h.kind_id                                          as "kind_id!",
               h.value,
               (EXTRACT(EPOCH FROM h.observed_at) * 1000)::bigint as "observed_ms!"
        FROM   user_name_history h
        JOIN   ids USING (user_id)
        WHERE  (h.guild_id = $2 OR h.guild_id IS NULL)
        ORDER  BY h.user_id, h.kind_id, h.observed_at
        "#,
        &ids,
        guild_id,
    )
    .fetch_all(pool)
    .await?;

    let mut histories: HashMap<i64, UserHistory> = HashMap::new();
    for r in history_rows {
        let entry = histories.entry(r.user_id).or_default();
        let bucket = match r.kind_id {
            KIND_NICKNAME => &mut entry.nickname,
            KIND_GLOBAL_NAME => &mut entry.global_name,
            KIND_USERNAME => &mut entry.username,
            _ => continue,
        };
        bucket.push((r.observed_ms, r.value));
    }

    // Current-values fallback for users with no predating history row.
    let current_rows = sqlx::query!(
        r#"
        SELECT un.user_id                                          as "user_id!",
               COALESCE(nn.nickname, un.global_name, un.username) as display_name
        FROM   user_names un
        LEFT JOIN user_nicknames nn
          ON nn.user_id = un.user_id AND nn.guild_id = $2
        WHERE un.user_id = ANY($1)
        "#,
        &ids,
        guild_id,
    )
    .fetch_all(pool)
    .await?;

    let current: HashMap<i64, String> = current_rows
        .into_iter()
        .filter_map(|r| r.display_name.map(|n| (r.user_id, n)))
        .collect();

    for ch in channels.iter_mut() {
        for dir in ch.dirs.iter_mut() {
            if let Some(months) = dir.months.as_mut() {
                for files in months.values_mut().flatten() {
                    for f in files.iter_mut() {
                        let Some(uid) = f.user_id.as_deref().and_then(|s| s.parse::<i64>().ok())
                        else {
                            continue;
                        };
                        let ts = f.start_ts_ms;
                        let resolved = ts.and_then(|t| {
                            histories.get(&uid).and_then(|h| {
                                latest_at(&h.nickname, t)
                                    .or_else(|| latest_at(&h.global_name, t))
                                    .or_else(|| latest_at(&h.username, t))
                                    .map(|s| s.to_string())
                            })
                        });
                        if resolved.is_some() {
                            f.display_name = resolved;
                            continue;
                        }
                        // Fallback path — record so we can watch the gap close.
                        let n = FALLBACK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        tracing::debug!(
                            target: "display_name_fallback",
                            user_id = uid,
                            start_ts_ms = ts,
                            guild_id = guild_id,
                            "no history predates start_ts; using current value"
                        );
                        if n % 1000 == 0 {
                            tracing::info!(
                                target: "display_name_fallback",
                                count = n,
                                "display-name fallback fired N times"
                            );
                        }
                        f.display_name = current.get(&uid).cloned();
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ms: i64, v: &str) -> (i64, Option<String>) {
        (ms, Some(v.to_string()))
    }

    #[test]
    fn picks_latest_predating_entry() {
        let events = vec![ev(100, "a"), ev(200, "b"), ev(300, "c")];
        assert_eq!(latest_at(&events, 250), Some("b"));
        assert_eq!(latest_at(&events, 300), Some("c"));
        assert_eq!(latest_at(&events, 99), None);
    }

    #[test]
    fn null_value_falls_through() {
        let events = vec![ev(100, "old"), (200, None)];
        // start_ts after the NULL clear → fall-through (None returned)
        assert_eq!(latest_at(&events, 250), None);
        // start_ts before the clear → still resolves to old value
        assert_eq!(latest_at(&events, 150), Some("old"));
    }

    #[test]
    fn coalesce_order_nick_then_global_then_username() {
        let h = UserHistory {
            nickname: vec![ev(100, "Nick")],
            global_name: vec![ev(50, "Global")],
            username: vec![ev(10, "uname")],
        };
        let pick = |ts: i64| {
            latest_at(&h.nickname, ts)
                .or_else(|| latest_at(&h.global_name, ts))
                .or_else(|| latest_at(&h.username, ts))
                .map(|s| s.to_string())
        };
        assert_eq!(pick(150), Some("Nick".into()));
        assert_eq!(pick(75), Some("Global".into()));
        assert_eq!(pick(20), Some("uname".into()));
        assert_eq!(pick(5), None);
    }
}

pub async fn get_channels_dir(
    guild_id: String,
    channel_hashset: HashSet<i64>,
) -> Result<Vec<Channels>, AppError> {
    let mut dirs_vec = Vec::new();
    for_channel_ids(guild_id, &mut dirs_vec, channel_hashset).await?;
    Ok(dirs_vec)
}

pub async fn for_channel_ids(
    guild_id: String,
    dirs_vec: &mut Vec<Channels>,
    channel_hashset: HashSet<i64>,
) -> Result<(), AppError> {
    let mut channel_ids = tokio::fs::read_dir(format!("{}{}", RECORDING_PATH, guild_id))
        .await
        .map_err(|err| {
            tracing::error!("{}", err);
            AppError::FileNotFound
        })?;

    loop {
        let entry = match channel_ids.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                tracing::error!("{}", err);
                continue;
            }
        };
        let channel = match entry.file_name().into_string() {
            Ok(s) => match s.parse::<i64>() {
                Ok(num) => num,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        if channel_hashset.contains(&channel) {
            let years = tokio::fs::read_dir(format!("{}{}/{}", RECORDING_PATH, guild_id, channel))
                .await
                .map_err(|err| {
                    tracing::error!("{}", err);
                    AppError::FileNotFound
                })?;

            let mut channels = Channels {
                channel_id: channel.to_string(),
                dirs: Vec::new(),
            };

            for_years(years, &guild_id, channel, &mut channels).await?;

            dirs_vec.push(channels);
        }
    }

    Ok(())
}

#[inline]
pub async fn for_years(
    mut years: tokio::fs::ReadDir,
    guild_id: &String,
    channel: i64,
    dirs_vec: &mut Channels,
) -> Result<(), AppError> {
    loop {
        let entry = match years.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                tracing::error!("{}", err);
                continue;
            }
        };
        let year_as_int = match entry.file_name().into_string() {
            Ok(s) => match s.parse::<i32>() {
                Ok(num) => num,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        let mut dirs = Directories {
            year: year_as_int,
            months: Some(HashMap::new()),
        };

        let months = tokio::fs::read_dir(format!(
            "{}{}/{}/{}",
            RECORDING_PATH, guild_id, channel, year_as_int
        ))
        .await
        .map_err(|err| {
            tracing::error!("{}", err);
            AppError::FileNotFound
        })?;

        for_months(months, &mut dirs, guild_id, channel, year_as_int).await?;

        dirs_vec.dirs.push(dirs);
    }
    Ok(())
}

#[inline]
pub async fn for_months(
    mut months: tokio::fs::ReadDir,
    dirs: &mut Directories,
    guild_id: &String,
    channel: i64,
    year_as_int: i32,
) -> Result<(), AppError> {
    loop {
        let entry = match months.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                error!("error for month: {}", err);
                continue;
            }
        };
        let month_as_string = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let month_as_int = match month_as_string.parse::<i32>() {
            Ok(m) if (1..=12).contains(&m) => m,
            _ => continue,
        };

        if let Some(months_map) = dirs.months.as_mut() {
            months_map.insert(month_as_int, Some(vec![]));
        }

        let entries = tokio::fs::read_dir(format!(
            "{}{}/{}/{}/{}",
            RECORDING_PATH, guild_id, channel, year_as_int, &month_as_string
        ))
        .await
        .map_err(|err| {
            tracing::error!("{}", err);
            AppError::FileNotFound
        })?;

        for_entry(entries, channel, dirs, month_as_int).await;
    }
    Ok(())
}

/// Live recordings for a guild, filtered to the channels the caller has read
/// access to. A recording is live only if its unfinished row has a fresh
/// recording heartbeat from a fresh, non-stopped bot instance.
#[utoipa::path(
    get,
    path = "/api/current/{guild_id}/live-stems",
    tag = "audio",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Currently live recording stems", body = [String]),
        (status = 400, description = "Invalid guild id", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/current/{guild_id}/live-stems")]
pub async fn get_live_stems(
    path: web::Path<String>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<sqlx::Pool<sqlx::Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let guild_id = path
        .into_inner()
        .parse::<i64>()
        .map_err(|_| AppError::InvalidParam("guild_id".into()))?;

    let permitted =
        crate::permissions::get_available_channels_for_user(&pool, guild_id, token.user_id).await?;

    let rows = sqlx::query!(
        "SELECT af.file_name, af.channel_id
           FROM audio_files af
          WHERE af.guild_id = $1
            AND af.end_ts IS NULL
            AND af.reaped IS FALSE
            AND EXISTS (
                SELECT 1
                  FROM bot_instances bi
                 WHERE bi.instance_id = af.recording_owner_instance_id
                   AND af.recording_heartbeat_at > now() - interval '120 seconds'
                   AND bi.heartbeat_at > now() - interval '120 seconds'
                   AND bi.state <> 'stopped'
            )",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;

    let stems: Vec<String> = rows
        .into_iter()
        .filter(|r| permitted.contains(&r.channel_id))
        .map(|r| r.file_name)
        .collect();

    Ok(HttpResponse::Ok().json(stems))
}

#[utoipa::path(
    get,
    path = "/api/current/{guild_id}",
    tag = "audio",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "Recording tree visible to current user", body = [Channels]),
        (status = 400, description = "Invalid guild id", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 404, description = "Recording directory not found", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/current/{guild_id}")]
pub async fn get_current_month_permission(
    path: web::Path<String>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<sqlx::Pool<sqlx::Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;

    let guild_id = path.into_inner();
    let guild_id_as_int = guild_id
        .parse::<i64>()
        .map_err(|_| AppError::InvalidParam("guild_id".into()))?;

    let permission_hashset =
        crate::permissions::get_available_channels_for_user(&pool, guild_id_as_int, token.user_id)
            .await?;

    let mut dirs_vec = get_channels_dir(guild_id, permission_hashset).await?;

    if let Err(e) = enrich_display_names(&pool, guild_id_as_int, &mut dirs_vec).await {
        tracing::error!("enrich_display_names failed: {}", e);
    }

    Ok(HttpResponse::Ok().json(dirs_vec))
}

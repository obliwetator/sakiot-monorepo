use crate::cast::ToI64;
use serenity::model::{guild::Guild, guild::Member, user::User};
use sqlx::{Pool, Postgres};
use tracing::{info, warn};

#[repr(i32)]
enum UserNameEventType {
    Username = 1,
    GlobalName = 2,
    Nickname = 3,
}

/// Record the latest observed names for a user. Each differing value also
/// appends a row to user_name_history so renames stay traceable.
pub async fn observe(pool: &Pool<Postgres>, guild_id: u64, user: &User, member: Option<&Member>) {
    let user_id = user.id.to_i64();
    let guild_id_i = guild_id.to_i64();
    let username = user.name.clone();
    let global_name = user.global_name.clone();

    let existing = sqlx::query!(
        "SELECT username, global_name FROM user_names WHERE user_id = $1",
        user_id
    )
    .fetch_optional(pool)
    .await;

    match existing {
        Ok(None) => {
            if let Err(e) = sqlx::query!(
                "INSERT INTO user_names (user_id, username, global_name) VALUES ($1, $2, $3)",
                user_id,
                username,
                global_name,
            )
            .execute(pool)
            .await
            {
                warn!("user_names initial insert failed for {}: {}", user_id, e);
            }

            push_history(
                pool,
                user_id,
                None,
                UserNameEventType::Username,
                Some(&username),
            )
            .await;
            if let Some(ref gn) = global_name {
                push_history(pool, user_id, None, UserNameEventType::GlobalName, Some(gn)).await;
            }
        }
        Ok(Some(row)) => {
            let username_changed = row.username != username;
            let global_changed = row.global_name != global_name;

            if username_changed || global_changed {
                if let Err(e) = sqlx::query!(
                    "UPDATE user_names SET username = $2, global_name = $3, updated_at = now()
                     WHERE user_id = $1",
                    user_id,
                    username,
                    global_name,
                )
                .execute(pool)
                .await
                {
                    warn!("user_names update failed for {}: {}", user_id, e);
                }

                if username_changed {
                    push_history(
                        pool,
                        user_id,
                        None,
                        UserNameEventType::Username,
                        Some(&username),
                    )
                    .await;
                }
                if global_changed {
                    push_history(
                        pool,
                        user_id,
                        None,
                        UserNameEventType::GlobalName,
                        global_name.as_deref(),
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            warn!("user_names lookup failed for {}: {}", user_id, e);
            return;
        }
    }

    if let Some(m) = member {
        observe_nickname(pool, user_id, guild_id_i, m.nick.as_deref()).await;
    }
}

async fn observe_nickname(
    pool: &Pool<Postgres>,
    user_id: i64,
    guild_id: i64,
    nickname: Option<&str>,
) {
    let existing = sqlx::query!(
        "SELECT nickname FROM user_nicknames WHERE user_id = $1 AND guild_id = $2",
        user_id,
        guild_id
    )
    .fetch_optional(pool)
    .await;

    match existing {
        Ok(None) => {
            if let Err(e) = sqlx::query!(
                "INSERT INTO user_nicknames (user_id, guild_id, nickname) VALUES ($1, $2, $3)",
                user_id,
                guild_id,
                nickname,
            )
            .execute(pool)
            .await
            {
                warn!(
                    "user_nicknames insert failed for {} @ {}: {}",
                    user_id, guild_id, e
                );
            }
            if nickname.is_some() {
                push_history(
                    pool,
                    user_id,
                    Some(guild_id),
                    UserNameEventType::Nickname,
                    nickname,
                )
                .await;
            }
        }
        Ok(Some(row)) => {
            if row.nickname.as_deref() != nickname {
                if let Err(e) = sqlx::query!(
                    "UPDATE user_nicknames SET nickname = $3, updated_at = now()
                     WHERE user_id = $1 AND guild_id = $2",
                    user_id,
                    guild_id,
                    nickname,
                )
                .execute(pool)
                .await
                {
                    warn!(
                        "user_nicknames update failed for {} @ {}: {}",
                        user_id, guild_id, e
                    );
                }
                push_history(
                    pool,
                    user_id,
                    Some(guild_id),
                    UserNameEventType::Nickname,
                    nickname,
                )
                .await;
            }
        }
        Err(e) => {
            warn!(
                "user_nicknames lookup failed for {} @ {}: {}",
                user_id, guild_id, e
            );
        }
    }
}

/// Seed name tables from the member cache of the given guilds. Called once on
/// cache_ready. Bots are skipped.
pub async fn seed_from_guilds(pool: &Pool<Postgres>, guilds: &[Guild]) {
    let mut count = 0usize;
    for guild in guilds {
        let guild_id = guild.id.get();
        for member in guild.members.values() {
            if member.user.bot {
                continue;
            }
            observe(pool, guild_id, &member.user, Some(member)).await;
            count += 1;
        }
    }
    info!("Seeded user_names for {} members", count);
}

async fn push_history(
    pool: &Pool<Postgres>,
    user_id: i64,
    guild_id: Option<i64>,
    kind: UserNameEventType,
    value: Option<&str>,
) {
    if let Err(e) = sqlx::query!(
        "INSERT INTO user_name_history (user_id, guild_id, kind_id, value)
         VALUES ($1, $2, $3, $4)",
        user_id,
        guild_id,
        kind as i32,
        value,
    )
    .execute(pool)
    .await
    {
        warn!("user_name_history insert failed for {}: {}", user_id, e);
    }
}

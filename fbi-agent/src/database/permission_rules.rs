//! Sakiot's own permission rules (`permission_rules`), which the dashboard's
//! rule editor writes and every command that asks the evaluator applies.

use sakiot_permissions::{Capabilities, Rule, Rules, Subject};
use sqlx::{Pool, Postgres};

use super::DbResult;

/// Every rule of `guild_id`. A guild has few, and the evaluator picks the
/// ones that reach the member; it ignores those of channels it does not
/// know.
pub async fn load(pool: &Pool<Postgres>, guild_id: i64) -> DbResult<Rules> {
    let rows = sqlx::query!(
        "SELECT channel_id, subject_kind, subject_id, allow, deny
           FROM permission_rules
          WHERE guild_id = $1",
        guild_id
    )
    .fetch_all(pool)
    .await?;
    let mut rules = Rules::default();
    for row in rows {
        // The table's constraints admit nothing else.
        let Some(subject) = Subject::from_stored(&row.subject_kind, row.subject_id) else {
            continue;
        };
        let rule = Rule {
            allow: Capabilities::from_stored(row.allow),
            deny: Capabilities::from_stored(row.deny),
        };
        rules.set(row.channel_id, subject, rule);
    }
    Ok(rules)
}

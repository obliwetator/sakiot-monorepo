//! Loads a guild's permission facts from the cache tables the bot keeps.

use std::collections::HashMap;

use sakiot_permissions::{ChannelFacts, GuildFacts, MemberFacts, Overwrite, Permissions, Roster};
use sqlx::types::Json;

use crate::errors::AppError;

#[derive(Debug, serde::Deserialize)]
struct RoleRow {
    role_id: i64,
    permissions: i64,
}

#[derive(Debug, serde::Deserialize)]
struct MemberRow {
    user_id: i64,
    listed: bool,
    roles: Vec<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct SeedRow {
    user_id: i64,
    owner: bool,
}

#[derive(Debug, serde::Deserialize)]
struct ChannelRow {
    channel_id: i64,
    overwrites: Vec<OverwriteRow>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum OverwriteKind {
    Role,
    User,
}

#[derive(Debug, serde::Deserialize)]
struct OverwriteRow {
    target_id: i64,
    kind: OverwriteKind,
    allow: i64,
    deny: i64,
}

/// Everything the evaluator needs about `guild_id` for `users` and for
/// `roles` (a previewed role), in one statement, so it is one consistent
/// snapshot without a transaction:
///
/// - the owner, whether the bot's member list was ever complete, and
///   whether the bot is in the guild;
/// - `@everyone`, the guild's roles the users hold, and `roles` when they
///   are the guild's;
/// - every voice and stage channel with its `@everyone` overwrite and the
///   overwrites for exactly those roles and users;
/// - each user's listing and roles, and their dev seeding (`user_guilds`).
pub(super) async fn load<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    guild_id: i64,
    users: &[i64],
    roles: &[i64],
) -> Result<GuildFacts, AppError> {
    crate::server_timing::measure("perm", async {
        let row = sqlx::query!(
            r#"WITH held AS (
                   SELECT ur.user_id, ur.role_id
                     FROM user_roles ur
                     JOIN roles r ON r.role_id = ur.role_id
                    WHERE r.guild_id = $1
                      AND ur.user_id = ANY($2::bigint[])
               ),
               loaded_roles AS (
                   SELECT role_id, permission
                     FROM roles
                    WHERE guild_id = $1
                      AND (role_id = $1
                           OR role_id IN (SELECT role_id FROM held)
                           OR role_id = ANY($3::bigint[]))
               )
               SELECT
                   (SELECT owner_id FROM guilds WHERE id = $1) AS owner_id,
                   COALESCE(
                       (SELECT roster_complete_at IS NOT NULL
                          FROM guild_projection_state
                         WHERE guild_id = $1),
                       false
                   ) AS "roster_complete!",
                   EXISTS (SELECT 1 FROM guilds_present WHERE guild_id = $1) AS "bot_present!",
                   (SELECT COALESCE(json_agg(json_build_object(
                               'role_id', role_id,
                               'permissions', permission)), '[]')
                      FROM loaded_roles) AS "roles!: Json<Vec<RoleRow>>",
                   (SELECT COALESCE(json_agg(json_build_object(
                               'user_id', u.user_id,
                               'listed', EXISTS (
                                   SELECT 1 FROM guild_members gm
                                    WHERE gm.guild_id = $1 AND gm.user_id = u.user_id),
                               'roles', (SELECT COALESCE(json_agg(h.role_id), '[]')
                                           FROM held h
                                          WHERE h.user_id = u.user_id))), '[]')
                      FROM (SELECT DISTINCT unnest($2::bigint[]) AS user_id) u
                   ) AS "members!: Json<Vec<MemberRow>>",
                   (SELECT COALESCE(json_agg(json_build_object(
                               'user_id', user_id,
                               'owner', owner)), '[]')
                      FROM user_guilds
                     WHERE id = $1 AND user_id = ANY($2::bigint[])
                   ) AS "seeds!: Json<Vec<SeedRow>>",
                   (SELECT COALESCE(json_agg(json_build_object(
                               'channel_id', c.channel_id,
                               'overwrites', (
                                   SELECT COALESCE(json_agg(json_build_object(
                                              'target_id', cp.target_id,
                                              'kind', cp.kind,
                                              'allow', cp.allow,
                                              'deny', cp.deny)), '[]')
                                     FROM channel_permissions cp
                                    WHERE cp.channel_id = c.channel_id
                                      AND (cp.target_id = $1
                                           OR (cp.kind = 'role'
                                               AND cp.target_id IN (
                                                   SELECT role_id FROM loaded_roles))
                                           OR (cp.kind = 'user'
                                               AND cp.target_id = ANY($2::bigint[]))))
                           )), '[]')
                      FROM channels c
                     WHERE c.guild_id = $1
                       AND c.type IN (2, 13)
                   ) AS "channels!: Json<Vec<ChannelRow>>""#,
            guild_id,
            users,
            roles,
        )
        .fetch_one(executor)
        .await?;

        let mut facts = GuildFacts::new(guild_id);
        facts.owner_id = row.owner_id;
        facts.roster = if row.roster_complete {
            Roster::Complete
        } else {
            Roster::Incomplete
        };
        facts.bot_present = row.bot_present;
        for role in row.roles.0 {
            let permissions = Permissions::from_bits_retain(role.permissions);
            if role.role_id == guild_id {
                facts.everyone = Some(permissions);
            } else {
                facts.roles.insert(role.role_id, permissions);
            }
        }
        facts.members = row
            .members
            .0
            .into_iter()
            .map(|member| {
                let facts = MemberFacts {
                    listed: member.listed,
                    roles: member.roles,
                };
                (member.user_id, facts)
            })
            .collect();
        facts.seeds = row
            .seeds
            .0
            .into_iter()
            .map(|seed| (seed.user_id, seed.owner))
            .collect();
        facts.channels = row
            .channels
            .0
            .into_iter()
            .map(|channel| {
                (
                    channel.channel_id,
                    channel_facts(guild_id, channel.overwrites),
                )
            })
            .collect();
        Ok(facts)
    })
    .await
}

fn channel_facts(guild_id: i64, overwrites: Vec<OverwriteRow>) -> ChannelFacts {
    let mut facts = ChannelFacts {
        everyone: Overwrite::default(),
        roles: HashMap::new(),
        members: HashMap::new(),
    };
    for row in overwrites {
        let overwrite = Overwrite {
            allow: Permissions::from_bits_retain(row.allow),
            deny: Permissions::from_bits_retain(row.deny),
        };
        // `@everyone` is the role whose id is the guild's.
        if row.target_id == guild_id {
            facts.everyone = overwrite;
            continue;
        }
        let targets = match row.kind {
            OverwriteKind::Role => &mut facts.roles,
            OverwriteKind::User => &mut facts.members,
        };
        targets.insert(row.target_id, overwrite);
    }
    facts
}

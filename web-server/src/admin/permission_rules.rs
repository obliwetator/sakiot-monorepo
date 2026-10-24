//! Sakiot's own permission rules (`sakiot_permissions::Rules`): the rule
//! editor and the access explainer, for holders of `manage_permissions`.
//!
//! An editor sees the guild-wide rules and those of the voice and stage
//! channels they can see (the owner sees every channel), and changes only
//! what they hold where a rule applies, never taking `manage_permissions`
//! from themselves (`GuildFacts::edit_rule`).

use std::collections::BTreeMap;

use actix_web::{HttpRequest, HttpResponse, get, put, web};
use sakiot_permissions::{
    Access, Capabilities, Decision, Explanation, GuildFacts, Membership, Rule, RuleRefused, Source,
    Subject, Viewer,
};
use serde::{Deserialize, Serialize};
use serde_with::{As, DisplayFromStr};
use sqlx::{Pool, Postgres};

use crate::errors::AppError;
use crate::permissions::{Capability, load_facts, request_viewer};
use crate::snowflake_serde::SnowflakeAsStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    Everyone,
    Role,
    User,
}

/// Whom a rule is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RuleSubject {
    pub kind: SubjectKind,
    /// The role or user; absent for `everyone`.
    #[serde(
        default,
        with = "As::<Option<DisplayFromStr>>",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub id: Option<i64>,
}

impl RuleSubject {
    pub(crate) fn of(subject: Subject) -> Self {
        let (kind, id) = match subject {
            Subject::Everyone => (SubjectKind::Everyone, None),
            Subject::Role(role_id) => (SubjectKind::Role, Some(role_id)),
            Subject::User(user_id) => (SubjectKind::User, Some(user_id)),
        };
        Self { kind, id }
    }

    /// The subject, if well formed: an id for a role or a user (never
    /// `@everyone`'s, the guild's), none for everyone.
    pub(crate) fn subject(self, guild_id: i64) -> Result<Subject, AppError> {
        match (self.kind, self.id) {
            (SubjectKind::Everyone, None) => Ok(Subject::Everyone),
            (SubjectKind::Role, Some(role_id)) if role_id > 0 && role_id != guild_id => {
                Ok(Subject::Role(role_id))
            }
            (SubjectKind::User, Some(user_id)) if user_id > 0 => Ok(Subject::User(user_id)),
            _ => Err(AppError::InvalidParam("subject".into())),
        }
    }
}

/// One rule, as stored.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PermissionRule {
    /// Null for a server-wide rule.
    #[serde(with = "As::<Option<DisplayFromStr>>")]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub channel_id: Option<i64>,
    pub subject: RuleSubject,
    /// The role's or member's name; null for everyone, a role that is gone
    /// or a user never seen.
    pub name: Option<String>,
    pub allow: Vec<Capability>,
    pub deny: Vec<Capability>,
    /// Its role or channel is gone, so it reaches nobody; only deleting it
    /// is left.
    pub reaches_nobody: bool,
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub updated_by: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RuleChannel {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub name: String,
    /// What the viewer may allow or deny in this channel's rules.
    pub editable: Vec<Capability>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RuleRole {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub role_id: i64,
    pub name: String,
    #[schema(example = 16711680)]
    pub color: i64,
    pub color_secondary: Option<i64>,
    pub color_tertiary: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PermissionRules {
    /// What the viewer may allow or deny in server-wide rules.
    pub editable: Vec<Capability>,
    /// The voice and stage channels the viewer sees, by name.
    pub channels: Vec<RuleChannel>,
    /// The server's roles other than `@everyone`, for choosing whom a rule
    /// is for.
    pub roles: Vec<RuleRole>,
    /// The server-wide rules, those of the channels the viewer sees, and
    /// those whose channel is gone.
    pub rules: Vec<PermissionRule>,
}

/// One rule as the editor sets it. Allowing and denying nothing deletes it.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RuleChange {
    /// Null for a server-wide rule.
    #[serde(default, with = "As::<Option<DisplayFromStr>>")]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub channel_id: Option<i64>,
    pub subject: RuleSubject,
    pub allow: Vec<Capability>,
    pub deny: Vec<Capability>,
}

/// The viewer's access to the guild, which must include `manage_permissions`.
fn require_editor(facts: &GuildFacts, viewer: Viewer) -> Result<Access, AppError> {
    let access = facts
        .access(viewer)
        .map_err(|_| AppError::MembershipUnavailable)?;
    if !access.guild.contains(Capabilities::MANAGE_PERMISSIONS) {
        return Err(AppError::Forbidden);
    }
    Ok(access)
}

/// The facts of `guild_id` for `users` and `roles`, as one value the caller
/// may change.
async fn guild_facts<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    guild_id: i64,
    users: &[i64],
    roles: &[i64],
) -> Result<GuildFacts, AppError> {
    load_facts(executor, &[guild_id], users, roles)
        .await?
        .remove(&guild_id)
        .map(std::sync::Arc::unwrap_or_clone)
        .ok_or(AppError::InternalError)
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/permission-rules",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    responses(
        (status = 200, description = "The rules the viewer may see, and what they may change", body = PermissionRules),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "The viewer does not hold manage_permissions", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
        (status = 503, description = "The server's member list is still loading", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/permission-rules")]
pub async fn get_permission_rules(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let viewer = request_viewer(&req)?;
    let facts = guild_facts(pool.get_ref(), guild_id, &[viewer.user_id], &[]).await?;
    let access = require_editor(&facts, viewer)?;

    let channels: Vec<RuleChannel> = sqlx::query!(
        "SELECT channel_id, name FROM channels
          WHERE guild_id = $1 AND type IN (2, 13)
          ORDER BY name, channel_id",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?
    .into_iter()
    .filter(|row| access.sees_channel(row.channel_id))
    .map(|row| RuleChannel {
        channel_id: row.channel_id,
        name: row.name.unwrap_or_default(),
        editable: Capability::named(access.editable(Some(row.channel_id))),
    })
    .collect();
    let roles = sqlx::query_as!(
        RuleRole,
        "SELECT role_id, name, color, color_secondary, color_tertiary
           FROM roles
          WHERE guild_id = $1 AND role_id <> $1
          ORDER BY role_id",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?;
    let rules = sqlx::query!(
        r#"SELECT pr.channel_id, pr.subject_kind, pr.subject_id, pr.allow, pr.deny,
                  pr.updated_by, pr.updated_at,
                  CASE pr.subject_kind
                      WHEN 'role' THEN (SELECT r.name FROM roles r
                                         WHERE r.role_id = pr.subject_id
                                           AND r.guild_id = pr.guild_id)
                      WHEN 'user' THEN (SELECT COALESCE(nn.nickname, un.global_name, un.username)
                                          FROM user_names un
                                          LEFT JOIN user_nicknames nn
                                            ON nn.user_id = un.user_id
                                           AND nn.guild_id = pr.guild_id
                                         WHERE un.user_id = pr.subject_id)
                  END AS name,
                  (pr.subject_kind = 'role'
                   AND NOT EXISTS (SELECT 1 FROM roles r
                                    WHERE r.role_id = pr.subject_id
                                      AND r.guild_id = pr.guild_id))
                  OR (pr.channel_id IS NOT NULL
                      AND NOT EXISTS (SELECT 1 FROM channels c
                                       WHERE c.channel_id = pr.channel_id
                                         AND c.guild_id = pr.guild_id
                                         AND c.type IN (2, 13))) AS "reaches_nobody!"
             FROM permission_rules pr
            WHERE pr.guild_id = $1
            ORDER BY pr.channel_id NULLS FIRST,
                     array_position(ARRAY['everyone', 'role', 'user'], pr.subject_kind),
                     pr.subject_id"#,
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?
    .into_iter()
    // A rule in a channel hidden from the viewer stays hidden with it; one
    // whose channel is gone is theirs to tidy away.
    .filter(|row| {
        row.channel_id.is_none_or(|channel_id| {
            access.sees_channel(channel_id) || !facts.channels.contains_key(&channel_id)
        })
    })
    .map(|row| {
        let subject = match row.subject_kind.as_str() {
            "role" => SubjectKind::Role,
            "user" => SubjectKind::User,
            _ => SubjectKind::Everyone,
        };
        PermissionRule {
            channel_id: row.channel_id,
            subject: RuleSubject {
                kind: subject,
                id: row.subject_id,
            },
            name: row.name,
            allow: Capability::named(Capabilities::from_stored(row.allow)),
            deny: Capability::named(Capabilities::from_stored(row.deny)),
            reaches_nobody: row.reaches_nobody,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    })
    .collect();

    Ok(HttpResponse::Ok().json(PermissionRules {
        editable: Capability::named(access.editable(None)),
        channels,
        roles,
        rules,
    }))
}

#[utoipa::path(
    put,
    path = "/api/admin/guilds/{guild_id}/permission-rules",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id")),
    request_body = RuleChange,
    responses(
        (status = 204, description = "Rule set, or deleted when it allows and denies nothing"),
        (status = 400, description = "Invalid rule", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "The viewer may not make this change: no manage_permissions, a capability they do not hold there, or taking manage_permissions from themselves", body = crate::errors::ApiError),
        (status = 404, description = "The role or channel is not this server's", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
        (status = 503, description = "The server's member list is still loading", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[put("/admin/guilds/{guild_id}/permission-rules")]
pub async fn put_permission_rule(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    body: web::Json<RuleChange>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let viewer = request_viewer(&req)?;
    let subject = body.subject.subject(guild_id)?;
    let channel = body.channel_id;
    let rule = Rule {
        allow: Capability::set(&body.allow),
        deny: Capability::set(&body.deny),
    };
    let users: Vec<i64> = match subject {
        Subject::User(user_id) => vec![viewer.user_id, user_id],
        _ => vec![viewer.user_id],
    };
    let roles: Vec<i64> = match subject {
        Subject::Role(role_id) => vec![role_id],
        _ => Vec::new(),
    };

    let mut tx = pool.begin().await?;
    // One editor at a time per guild, so a rule checked against the others
    // is still checked against them when it lands.
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('permission_rules', $1))",
        guild_id
    )
    .execute(&mut *tx)
    .await?;
    let mut facts = guild_facts(&mut *tx, guild_id, &users, &roles).await?;
    let access = require_editor(&facts, viewer)?;
    // A channel hidden from the editor is as good as missing.
    if channel.is_some_and(|channel_id| {
        facts.channels.contains_key(&channel_id) && !access.sees_channel(channel_id)
    }) {
        return Err(AppError::NotFound);
    }
    if !rule.is_empty() {
        if let Subject::Role(role_id) = subject
            && !facts.has_role(role_id)
        {
            return Err(AppError::RoleNotFound);
        }
        if channel.is_some_and(|channel_id| !facts.channels.contains_key(&channel_id)) {
            return Err(AppError::NotFound);
        }
    }
    facts
        .edit_rule(viewer, channel, subject, rule)
        .map_err(refusal)?;

    let (kind, id) = subject.stored();
    if rule.is_empty() {
        sqlx::query!(
            "DELETE FROM permission_rules
              WHERE guild_id = $1
                AND channel_id IS NOT DISTINCT FROM $2
                AND subject_kind = $3
                AND subject_id IS NOT DISTINCT FROM $4",
            guild_id,
            channel,
            kind,
            id
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!(
            "INSERT INTO permission_rules
                (guild_id, channel_id, subject_kind, subject_id, allow, deny, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT ON CONSTRAINT permission_rules_one_per_subject DO UPDATE
             SET allow = EXCLUDED.allow,
                 deny = EXCLUDED.deny,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = now()",
            guild_id,
            channel,
            kind,
            id,
            rule.allow.stored(),
            rule.deny.stored(),
            viewer.user_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(HttpResponse::NoContent().finish())
}

fn refusal(refused: RuleRefused) -> AppError {
    match refused {
        RuleRefused::MembershipUnavailable => AppError::MembershipUnavailable,
        RuleRefused::NotAnEditor => AppError::Forbidden,
        RuleRefused::AllowsAndDenies => {
            AppError::BadRequest("A rule cannot both allow and deny the same capability.".into())
        }
        RuleRefused::ManagementInChannel => AppError::BadRequest(
            "Managing the server applies server-wide; set it in a server-wide rule.".into(),
        ),
        RuleRefused::NotHeld(beyond) => AppError::Refused(format!(
            "You can only change what you hold yourself where the rule applies, which excludes: {}.",
            Capability::named(beyond)
                .into_iter()
                .map(Capability::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        RuleRefused::OwnManagePermissions => {
            AppError::Refused("This would take away your own permission to edit rules.".into())
        }
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ExplainQuery {
    /// The member to explain. Exactly one of `user_id` and `role_id`.
    #[serde(default, with = "As::<Option<DisplayFromStr>>")]
    #[param(value_type = Option<String>)]
    pub user_id: Option<i64>,
    /// The role to explain, as a member holding only it; the server's id is
    /// `@everyone`.
    #[serde(default, with = "As::<Option<DisplayFromStr>>")]
    #[param(value_type = Option<String>)]
    pub role_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExplainedMembership {
    Owner,
    Member,
    /// No longer a member: their own recordings only, which no rule reaches.
    Former,
    /// Not a member: nothing, which no rule reaches.
    Outsider,
}

#[derive(Clone, Copy, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Discord's permissions, through the capability's default.
    Discord,
    /// A Sakiot rule.
    Rule,
    /// The owner keeps every management capability.
    Owner,
}

/// What decided a capability.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DecisionSource {
    pub kind: SourceKind,
    /// For a rule: its channel, null when server-wide.
    #[serde(with = "As::<Option<DisplayFromStr>>")]
    #[schema(value_type = Option<String>, example = "146638124288704513")]
    pub channel_id: Option<i64>,
    /// For a rule: whom it is for.
    pub subject: Option<RuleSubject>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CapabilityDecision {
    pub capability: Capability,
    pub held: bool,
    /// What Discord's permissions alone would give.
    pub discord: bool,
    pub source: DecisionSource,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ChannelDecisions {
    #[serde(with = "SnowflakeAsStr")]
    #[schema(value_type = String, example = "146638124288704513")]
    pub channel_id: i64,
    pub name: String,
    pub capabilities: Vec<CapabilityDecision>,
}

/// Why a member or a role holds what it holds.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccessExplanation {
    pub membership: ExplainedMembership,
    /// The management capabilities; empty when no rule reaches them.
    pub guild: Vec<CapabilityDecision>,
    /// The content capabilities in each voice and stage channel the viewer
    /// sees.
    pub channels: Vec<ChannelDecisions>,
}

#[utoipa::path(
    get,
    path = "/api/admin/guilds/{guild_id}/permission-rules/explain",
    tag = "admin",
    params(("guild_id" = i64, Path, description = "Discord guild id"), ExplainQuery),
    responses(
        (status = 200, description = "What the member or role holds, capability by capability, and what decided it", body = AccessExplanation),
        (status = 400, description = "Neither or both of user_id and role_id", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "The viewer does not hold manage_permissions", body = crate::errors::ApiError),
        (status = 404, description = "The role is not this server's", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
        (status = 503, description = "The server's member list is still loading", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/admin/guilds/{guild_id}/permission-rules/explain")]
pub async fn explain_access(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    path: web::Path<i64>,
    query: web::Query<ExplainQuery>,
) -> Result<HttpResponse, AppError> {
    let guild_id = path.into_inner();
    let viewer = request_viewer(&req)?;
    let (users, roles) = match (query.user_id, query.role_id) {
        (Some(user_id), None) => (vec![viewer.user_id, user_id], Vec::new()),
        (None, Some(role_id)) => (vec![viewer.user_id], vec![role_id]),
        _ => return Err(AppError::InvalidParam("user_id or role_id".into())),
    };
    let facts = guild_facts(pool.get_ref(), guild_id, &users, &roles).await?;
    let access = require_editor(&facts, viewer)?;
    let explanation = match (query.user_id, query.role_id) {
        (Some(user_id), _) => facts
            .explain(Viewer::discord(user_id))
            .map_err(|_| AppError::MembershipUnavailable)?,
        (_, Some(role_id)) => facts
            .explain_role(role_id)
            .map_err(|_| AppError::RoleNotFound)?,
        (None, None) => return Err(AppError::InvalidParam("user_id or role_id".into())),
    };

    let names: BTreeMap<i64, String> = sqlx::query!(
        "SELECT channel_id, name FROM channels WHERE guild_id = $1 AND type IN (2, 13)",
        guild_id
    )
    .fetch_all(pool.get_ref())
    .await?
    .into_iter()
    .map(|row| (row.channel_id, row.name.unwrap_or_default()))
    .collect();
    Ok(HttpResponse::Ok().json(explained(&explanation, &access, &names)))
}

/// `explanation` for the API: the channels the viewer sees, by name.
fn explained(
    explanation: &Explanation,
    viewer: &Access,
    names: &BTreeMap<i64, String>,
) -> AccessExplanation {
    let mut channels: Vec<ChannelDecisions> = explanation
        .channels
        .iter()
        .filter(|(channel_id, _)| viewer.sees_channel(**channel_id))
        .map(|(&channel_id, decisions)| ChannelDecisions {
            channel_id,
            name: names.get(&channel_id).cloned().unwrap_or_default(),
            capabilities: decisions.iter().filter_map(decision).collect(),
        })
        .collect();
    channels.sort_by(|a, b| a.name.cmp(&b.name).then(a.channel_id.cmp(&b.channel_id)));
    AccessExplanation {
        membership: match explanation.membership {
            Membership::Owner => ExplainedMembership::Owner,
            Membership::Member => ExplainedMembership::Member,
            Membership::Former => ExplainedMembership::Former,
            Membership::Outsider => ExplainedMembership::Outsider,
        },
        guild: explanation.guild.iter().filter_map(decision).collect(),
        channels,
    }
}

/// One decision for the API; every capability the evaluator decides has a
/// name.
fn decision(decision: &Decision) -> Option<CapabilityDecision> {
    let source = match decision.by {
        Source::Discord => DecisionSource {
            kind: SourceKind::Discord,
            channel_id: None,
            subject: None,
        },
        Source::Rule { channel, subject } => DecisionSource {
            kind: SourceKind::Rule,
            channel_id: channel,
            subject: Some(RuleSubject::of(subject)),
        },
        Source::Owner => DecisionSource {
            kind: SourceKind::Owner,
            channel_id: None,
            subject: None,
        },
    };
    Some(CapabilityDecision {
        capability: Capability::one(decision.capability)?,
        held: decision.held,
        discord: decision.discord,
        source,
    })
}

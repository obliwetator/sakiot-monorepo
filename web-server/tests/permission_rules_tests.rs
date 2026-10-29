mod support;

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use sakiot_permissions::Capabilities;
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::admin::permission_rules::{
    explain_access, get_permission_rules, put_permission_rule,
};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::clips::get_clips;
use web_server::members::get_role_view;
use web_server::permissions::{GuildAccess, Viewer};

const GUILD_ID: i64 = 1;
const OWNER_ID: i64 = 20;
/// Holds the Moderator role, whose MANAGE_GUILD makes them a rule editor.
const MODERATOR_ID: i64 = 30;
const VIP_ID: i64 = 40;
const MEMBER_ID: i64 = 10;
const MODERATOR_ROLE_ID: i64 = 1001;
const VIP_ROLE_ID: i64 = 1002;
const GONE_ROLE_ID: i64 = 1999;
/// Open to everyone.
const LOUNGE_ID: i64 = 100;
/// Hidden from everyone but the owner.
const STAFF_ID: i64 = 200;
const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const MANAGE_GUILD: i64 = 1 << 5;
const CSRF: &str = "csrf-test-token";

fn access_cookie_for(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
        "test-session".into(),
        CSRF.to_string(),
        &EncodingKey::from_secret(b"test_secret"),
    )?;
    Ok(format!("{ACCESS_TOKEN_COOKIE}={token}"))
}

fn access_keys() -> AccessKeys {
    AccessKeys {
        access_encode: EncodingKey::from_secret(b"test_secret"),
        refresh_encode: EncodingKey::from_secret(b"test_secret"),
        access_decode: DecodingKey::from_secret(b"test_secret"),
        refresh_decode: DecodingKey::from_secret(b"test_secret"),
    }
}

async fn seed(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2)",
        GUILD_ID,
        OWNER_ID
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name, color)
         VALUES ($1, $1, $2, '@everyone', 0),
                ($1, $3, $4, 'Moderator', 16711680),
                ($1, $5, 0, 'VIP', 255)",
        GUILD_ID,
        VIEW_CHANNEL | CONNECT,
        MODERATOR_ROLE_ID,
        MANAGE_GUILD,
        VIP_ROLE_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $3, 2, 'lounge'), ($2, $3, 2, 'staff')",
        LOUNGE_ID,
        STAFF_ID,
        GUILD_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $3)",
        STAFF_ID,
        GUILD_ID,
        VIEW_CHANNEL,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         SELECT $1, user_id, 'guild', NULL, false, 0, ARRAY[]::text[]
           FROM unnest($2::bigint[]) AS user_id",
        GUILD_ID,
        &[MEMBER_ID, MODERATOR_ID, VIP_ID],
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2), ($3, $4)",
        MODERATOR_ID,
        MODERATOR_ROLE_ID,
        VIP_ID,
        VIP_ROLE_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_names (user_id, username, global_name) VALUES ($1, 'member', 'Member')",
        MEMBER_ID
    )
    .execute(pool)
    .await?;
    support::complete_rosters_from_user_guilds(pool).await?;
    Ok(())
}

async fn holds(
    pool: &PgPool,
    user_id: i64,
    channel: Option<i64>,
    capability: Capabilities,
) -> Result<bool, Box<dyn std::error::Error>> {
    let access = GuildAccess::load(pool, GUILD_ID, Viewer::discord(user_id), None).await?;
    let access = access.access()?;
    Ok(match channel {
        None => access.guild.contains(capability),
        Some(channel) => access.allows(channel, capability),
    })
}

fn rule(channel: Option<i64>, subject: Value, allow: &[&str], deny: &[&str]) -> Value {
    json!({
        "channel_id": channel.map(|channel| channel.to_string()),
        "subject": subject,
        "allow": allow,
        "deny": deny,
    })
}

fn role(role_id: i64) -> Value {
    json!({ "kind": "role", "id": role_id.to_string() })
}

fn user(user_id: i64) -> Value {
    json!({ "kind": "user", "id": user_id.to_string() })
}

fn everyone() -> Value {
    json!({ "kind": "everyone" })
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn an_editor_changes_only_what_they_hold_and_never_their_own_editing(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed(&pool).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_permission_rules)
                    .service(put_permission_rule)
                    .service(explain_access),
            ),
    )
    .await;
    let uri = format!("/api/admin/guilds/{GUILD_ID}/permission-rules");
    let put = async |user_id: i64,
                     body: Value|
           -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let request = test::TestRequest::put()
            .uri(&uri)
            .insert_header(("Cookie", access_cookie_for(user_id)?))
            .insert_header(("X-CSRF-Token", CSRF))
            .set_json(body)
            .to_request();
        let response = test::call_service(&app, request).await;
        let status = response.status();
        let body = test::read_body(response).await;
        Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    };
    let get = async |user_id: i64,
                     path: &str|
           -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let request = test::TestRequest::get()
            .uri(&format!("{uri}{path}"))
            .insert_header(("Cookie", access_cookie_for(user_id)?))
            .to_request();
        let response = test::call_service(&app, request).await;
        let status = response.status();
        let body = test::read_body(response).await;
        Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    };

    // Only holders of manage_permissions see the rules.
    assert_eq!(get(MEMBER_ID, "").await?.0, StatusCode::FORBIDDEN);
    assert_eq!(
        put(MEMBER_ID, rule(None, everyone(), &[], &["stamp"]))
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, editor) = get(MODERATOR_ID, "").await?;
    assert_eq!(status, StatusCode::OK);
    // The moderator sees only the lounge, and may change guild-wide only
    // what reaches no channel they lack: management alone, since everything
    // in a channel takes seeing it, and not who edits rules, since that
    // lifts every channel rule for whoever it reaches.
    let channels: Vec<&str> = editor["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|channel| channel["name"].as_str())
        .collect();
    assert_eq!(channels, ["lounge"]);
    assert_eq!(
        editor["editable"],
        json!([
            "manage_settings",
            "preview_roles",
            "delete_recordings",
            "moderate_clips",
        ])
    );
    assert_eq!(
        editor["roles"].as_array().map(|roles| roles
            .iter()
            .map(|role| role["name"].clone())
            .collect::<Vec<_>>()),
        Some(vec![json!("Moderator"), json!("VIP")])
    );
    let (_, owner) = get(OWNER_ID, "").await?;
    assert_eq!(owner["channels"].as_array().map(Vec::len), Some(2));

    // Within what they hold, a rule applies at once.
    assert_eq!(
        put(
            MODERATOR_ID,
            rule(Some(LOUNGE_ID), everyone(), &[], &["jam"])
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(!holds(&pool, MEMBER_ID, Some(LOUNGE_ID), Capabilities::JAM).await?);
    assert!(
        holds(
            &pool,
            MEMBER_ID,
            Some(LOUNGE_ID),
            Capabilities::LISTEN_OTHERS
        )
        .await?
    );
    assert_eq!(
        put(
            MODERATOR_ID,
            rule(None, role(VIP_ROLE_ID), &["delete_recordings"], &[])
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(holds(&pool, VIP_ID, None, Capabilities::DELETE_RECORDINGS).await?);

    // Not past it: listening guild-wide would reach the staff channel.
    let (status, refused) = put(
        MODERATOR_ID,
        rule(None, role(VIP_ROLE_ID), &["listen_others"], &[]),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("listen_others")),
        "{refused}"
    );
    // A hidden channel is as good as missing.
    assert_eq!(
        put(
            MODERATOR_ID,
            rule(Some(STAFF_ID), user(VIP_ID), &[], &["stamp"])
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    // Nor who edits rules, while a channel is closed to them.
    let (status, refused) = put(
        MODERATOR_ID,
        rule(None, role(VIP_ROLE_ID), &["manage_permissions"], &[]),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("manage_permissions")),
        "{refused}"
    );
    // Malformed rules.
    for invalid in [
        rule(Some(LOUNGE_ID), everyone(), &["manage_settings"], &[]),
        rule(None, everyone(), &["stamp"], &["stamp"]),
        rule(None, role(GUILD_ID), &["stamp"], &[]),
        rule(
            None,
            json!({ "kind": "everyone", "id": "5" }),
            &["stamp"],
            &[],
        ),
    ] {
        assert_eq!(put(OWNER_ID, invalid).await?.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        put(OWNER_ID, rule(None, role(GONE_ROLE_ID), &["stamp"], &[]))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );

    // The owner changes anything, hidden channels included; the moderator
    // never sees that rule.
    assert_eq!(
        put(
            OWNER_ID,
            rule(
                Some(STAFF_ID),
                user(MEMBER_ID),
                &["view_presence", "listen_others"],
                &[]
            )
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        holds(
            &pool,
            MEMBER_ID,
            Some(STAFF_ID),
            Capabilities::LISTEN_OTHERS
        )
        .await?
    );
    let channel_rules = |rules: &Value| -> Vec<Value> {
        rules["rules"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|rule| rule["channel_id"].clone())
            .collect()
    };
    assert_eq!(
        channel_rules(&get(OWNER_ID, "").await?.1),
        [
            Value::Null,
            json!(LOUNGE_ID.to_string()),
            json!(STAFF_ID.to_string())
        ]
    );
    assert_eq!(
        channel_rules(&get(MODERATOR_ID, "").await?.1),
        [Value::Null, json!(LOUNGE_ID.to_string())]
    );
    let (_, listed) = get(OWNER_ID, "").await?;
    let staff_rule = &listed["rules"][2];
    assert_eq!(staff_rule["subject"], user(MEMBER_ID));
    assert_eq!(staff_rule["name"], "Member");
    assert_eq!(
        staff_rule["allow"],
        json!(["view_presence", "listen_others"])
    );
    assert_eq!(staff_rule["updated_by"], OWNER_ID.to_string());

    // A rule whose role is gone reaches nobody, and any editor may delete it.
    sqlx::query!(
        "INSERT INTO permission_rules
            (guild_id, channel_id, subject_kind, subject_id, allow, deny, updated_by)
         VALUES ($1, NULL, 'role', $2, $3, 0, $4)",
        GUILD_ID,
        GONE_ROLE_ID,
        Capabilities::LISTEN_OTHERS.stored(),
        OWNER_ID,
    )
    .execute(&pool)
    .await?;
    let (_, listed) = get(MODERATOR_ID, "").await?;
    let gone = listed["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|rule| rule["subject"] == role(GONE_ROLE_ID))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(gone["reaches_nobody"], true);
    assert_eq!(gone["name"], Value::Null);
    assert_eq!(
        put(MODERATOR_ID, rule(None, role(GONE_ROLE_ID), &[], &[]))
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    // Allowing and denying nothing deletes a rule. No rule takes from an
    // editor, so the moderator's `@everyone` deny left them jam, and they
    // may undo it.
    assert!(holds(&pool, MODERATOR_ID, Some(LOUNGE_ID), Capabilities::JAM).await?);
    assert_eq!(
        put(MODERATOR_ID, rule(Some(LOUNGE_ID), everyone(), &[], &[]))
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(holds(&pool, MEMBER_ID, Some(LOUNGE_ID), Capabilities::JAM).await?);
    let remaining = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM permission_rules WHERE guild_id = $1"#,
        GUILD_ID
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(remaining, 2, "the VIP and staff rules");

    // The explainer: what decided each capability, in the channels the
    // viewer sees.
    let (status, explained) = get(OWNER_ID, &format!("/explain?user_id={MEMBER_ID}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(explained["membership"], "member");
    let staff = explained["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|channel| channel["name"] == "staff")
        .cloned()
        .unwrap_or(Value::Null);
    let listen = staff["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|decision| decision["capability"] == "listen_others")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        listen,
        json!({
            "capability": "listen_others",
            "held": true,
            "discord": false,
            "source": {
                "kind": "rule",
                "channel_id": STAFF_ID.to_string(),
                "subject": user(MEMBER_ID),
            },
        })
    );
    let (_, explained) = get(MODERATOR_ID, &format!("/explain?user_id={MEMBER_ID}")).await?;
    assert_eq!(explained["channels"].as_array().map(Vec::len), Some(1));
    let (status, explained) = get(MODERATOR_ID, &format!("/explain?role_id={VIP_ROLE_ID}")).await?;
    assert_eq!(status, StatusCode::OK);
    let deletes = explained["guild"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|decision| decision["capability"] == "delete_recordings")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(deletes["held"], true);
    assert_eq!(deletes["source"]["subject"], role(VIP_ROLE_ID));
    let (_, explained) = get(OWNER_ID, &format!("/explain?user_id={OWNER_ID}")).await?;
    assert_eq!(explained["membership"], "owner");
    assert_eq!(
        get(OWNER_ID, "/explain").await?.0,
        StatusCode::BAD_REQUEST,
        "neither a user nor a role"
    );
    assert_eq!(
        get(OWNER_ID, &format!("/explain?role_id={GONE_ROLE_ID}"))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(MEMBER_ID, &format!("/explain?user_id={MEMBER_ID}"))
            .await?
            .0,
        StatusCode::FORBIDDEN
    );

    // Given the staff channel, the moderator holds everything everywhere and
    // may change who edits rules, but never take it from themselves.
    assert_eq!(
        put(
            OWNER_ID,
            rule(
                Some(STAFF_ID),
                user(MODERATOR_ID),
                &[
                    "view_presence",
                    "listen_others",
                    "create_clips",
                    "jam",
                    "stamp"
                ],
                &[]
            )
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(
            MODERATOR_ID,
            rule(None, role(VIP_ROLE_ID), &["manage_permissions"], &[])
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, refused) = put(
        MODERATOR_ID,
        rule(None, role(MODERATOR_ROLE_ID), &[], &["manage_permissions"]),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("your own")),
        "{refused}"
    );
    assert!(holds(&pool, MODERATOR_ID, None, Capabilities::MANAGE_PERMISSIONS).await?);
    Ok(())
}

/// Denying `@everyone` the sight of a channel takes everything else there
/// too, as Discord's View Channel does: its clips leave a preview of
/// `@everyone`, and the role view says why. No rule takes from the owner,
/// who keeps the channel.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_channel_nobody_sees_takes_its_clips_from_a_preview(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed(&pool).await?;
    sqlx::query!(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, channel_ids, speaker_ids, user_id,
             saved_file_name, start_time)
         VALUES ('lounge-clip', $1, $2, ARRAY[$2::bigint], ARRAY[$3::bigint], $3,
                 'lounge/clip.ogg', 0)",
        GUILD_ID,
        LOUNGE_ID,
        MEMBER_ID,
    )
    .execute(&pool)
    .await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(put_permission_rule)
                    .service(get_clips)
                    .service(get_role_view),
            ),
    )
    .await;
    let get = async |uri: String| -> Result<Value, Box<dyn std::error::Error>> {
        let request = test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Cookie", access_cookie_for(OWNER_ID)?))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        Ok(test::read_body_json(response).await)
    };
    let clips = format!("/api/audio/clips/{GUILD_ID}?as_role={GUILD_ID}");
    let listed = get(clips.clone()).await?;
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let request = test::TestRequest::put()
        .uri(&format!("/api/admin/guilds/{GUILD_ID}/permission-rules"))
        .insert_header(("Cookie", access_cookie_for(OWNER_ID)?))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(rule(Some(LOUNGE_ID), everyone(), &[], &["view_presence"]))
        .to_request();
    assert!(
        test::call_service(&app, request)
            .await
            .status()
            .is_success()
    );

    assert_eq!(get(clips).await?, json!([]));
    assert!(
        holds(
            &pool,
            OWNER_ID,
            Some(LOUNGE_ID),
            Capabilities::LISTEN_OTHERS
        )
        .await?
    );
    let view = get(format!(
        "/api/admin/guilds/{GUILD_ID}/roles/{GUILD_ID}/channels"
    ))
    .await?;
    let lounge = view["channels"]
        .as_array()
        .and_then(|channels| {
            channels
                .iter()
                .find(|channel| channel["channel_id"] == json!(LOUNGE_ID.to_string()))
        })
        .ok_or("the lounge listed")?;
    let decided = |capability: &str| {
        lounge["capabilities"]
            .as_array()
            .and_then(|decisions| {
                decisions
                    .iter()
                    .find(|decision| decision["capability"] == capability)
            })
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        decided("view_presence"),
        json!({
            "capability": "view_presence",
            "held": false,
            "discord": true,
            "source": {
                "kind": "rule",
                "channel_id": LOUNGE_ID.to_string(),
                "subject": { "kind": "everyone" },
            },
        })
    );
    for capability in ["listen_others", "create_clips", "jam", "stamp"] {
        assert_eq!(
            decided(capability),
            json!({
                "capability": capability,
                "held": false,
                "discord": true,
                "source": { "kind": "unseen", "channel_id": null, "subject": null },
            })
        );
    }
    Ok(())
}

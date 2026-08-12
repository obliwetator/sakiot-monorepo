use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::audio::get_current_month_permission;
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::members::{get_guild_roles, get_role_members, get_role_view};

const OWNER_ID: i64 = 20;
const MEMBER_ID: i64 = 10;
const DEV_ACCOUNT_ID: i64 = 99;
const GUILD_ID: i64 = 1;
const OTHER_GUILD_ID: i64 = 2;
const MODERATOR_ROLE_ID: i64 = 1001;
const VIP_ROLE_ID: i64 = 1002;
const CSRF: &str = "csrf-test-token";

fn access_cookie_for(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
        CSRF.to_string(),
        &EncodingKey::from_secret(b"test_secret"),
    )?;
    Ok(format!("{ACCESS_TOKEN_COOKIE}={token}"))
}

fn dev_access_cookie_for(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
        AuthKind::Dev,
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

async fn seed_members_data(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2), ($3, $2)",
        GUILD_ID,
        OWNER_ID,
        OTHER_GUILD_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name, color, color_secondary)
         VALUES
            ($1, $1, 1049600, '@everyone', 0, NULL),
            ($1, $2, 8, 'Moderator', 16711680, 65280),
            ($1, $3, 1024, 'VIP', 255, NULL),
            ($4, $4, 1049600, '@everyone', 0, NULL)",
        GUILD_ID,
        MODERATOR_ROLE_ID,
        VIP_ROLE_ID,
        OTHER_GUILD_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         VALUES ($1, $2, 'allowed guild', NULL, false, 0, ARRAY[]::text[])",
        GUILD_ID,
        MEMBER_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id)
         VALUES
            (30, $1), (50, $1), (40, $2), (50, $2), (60, $1)",
        MODERATOR_ROLE_ID,
        VIP_ROLE_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_names (user_id, username, global_name)
         VALUES (30, 'alice', NULL), (40, 'bob', 'Bobby'), (50, 'carol', NULL)"
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_nicknames (user_id, guild_id, nickname)
         VALUES (40, $1, 'BobbyNick')",
        GUILD_ID,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn roles_listed_with_member_counts_for_manager(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_guild_roles)
                    .service(get_role_members),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/admin/guilds/{GUILD_ID}/roles"))
        .insert_header(("Cookie", cookie))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = test::read_body_json(response).await;
    let roles = body.as_array().unwrap();
    assert_eq!(roles.len(), 3);
    let by_id = |id: i64| -> Value {
        roles
            .iter()
            .find(|r| r["role_id"] == json!(id.to_string()))
            .cloned()
            .expect("role present")
    };
    assert_eq!(by_id(MODERATOR_ROLE_ID)["name"], "Moderator");
    assert_eq!(by_id(MODERATOR_ROLE_ID)["member_count"], 3);
    assert_eq!(by_id(MODERATOR_ROLE_ID)["color"], 16711680);
    assert_eq!(by_id(MODERATOR_ROLE_ID)["color_secondary"], 65280);
    assert_eq!(by_id(MODERATOR_ROLE_ID)["color_tertiary"], Value::Null);
    assert_eq!(by_id(VIP_ROLE_ID)["name"], "VIP");
    assert_eq!(by_id(VIP_ROLE_ID)["member_count"], 2);
    assert_eq!(by_id(VIP_ROLE_ID)["color"], 255);
    assert_eq!(by_id(VIP_ROLE_ID)["color_secondary"], Value::Null);
    assert_eq!(by_id(GUILD_ID)["name"], "@everyone");
    assert_eq!(by_id(GUILD_ID)["member_count"], 0);
    assert_eq!(by_id(GUILD_ID)["color"], 0);
    assert_eq!(
        roles.last().expect("ordered")["role_id"],
        json!(GUILD_ID.to_string())
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn roles_endpoints_forbidden_for_plain_member(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;

    let cookie = access_cookie_for(MEMBER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_guild_roles)
                    .service(get_role_members),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/admin/guilds/{GUILD_ID}/roles"))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::FORBIDDEN
    );

    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{MODERATOR_ROLE_ID}/members"
        ))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_members_resolve_names_with_nickname_precedence(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_role_members),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{MODERATOR_ROLE_ID}/members"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    let members = body.as_array().unwrap();
    assert_eq!(members.len(), 3);
    let by_id = |id: i64| -> Value {
        members
            .iter()
            .find(|m| m["user_id"] == json!(id.to_string()))
            .cloned()
            .expect("member present")
    };
    assert_eq!(by_id(30)["name"], "alice");
    assert_eq!(by_id(50)["name"], "carol");
    assert_eq!(by_id(60)["name"], Value::Null);

    // Nickname beats global_name beats username in this guild.
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{VIP_ROLE_ID}/members"
        ))
        .insert_header(("Cookie", cookie))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    let members = body.as_array().unwrap();
    let bob = members
        .iter()
        .find(|m| m["user_id"] == json!("40".to_string()))
        .expect("bob present");
    assert_eq!(bob["name"], "BobbyNick");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn dev_account_can_manage_guilds_granted_owner_snapshot(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;
    // The dev login is not the guild owner and has no roles in the role cache,
    // but local tooling (seed / fixture import) granted it owner = true in
    // user_guilds, which the frontend's isGuildAdmin also trusts.
    sqlx::query!(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         VALUES ($1, $2, 'granted guild', NULL, true, 8, ARRAY[]::text[])",
        GUILD_ID,
        DEV_ACCOUNT_ID,
    )
    .execute(&pool)
    .await?;

    let cookie = dev_access_cookie_for(DEV_ACCOUNT_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_guild_roles)
                    .service(get_role_members),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/admin/guilds/{GUILD_ID}/roles"))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);

    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{MODERATOR_ROLE_ID}/members"
        ))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::OK
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn dev_account_without_grant_is_still_forbidden(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;
    // No user_guilds row for the dev account: the bypass must not open up
    // guilds the local tooling never granted.
    let cookie = dev_access_cookie_for(DEV_ACCOUNT_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_guild_roles),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/admin/guilds/{GUILD_ID}/roles"))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_members_unknown_or_foreign_role_returns_not_found(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_members_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_role_members),
            ),
    )
    .await;

    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/999999/members"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::NOT_FOUND
    );

    // OTHER_GUILD_ID's @everyone role is not this guild's role.
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{OTHER_GUILD_ID}/members"
        ))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

// ---- view-as-role preview ----

const VIEW_ROLE_ID: i64 = 1003;
const ADMIN_ROLE_ID: i64 = 1004;
const CHANNEL_A: i64 = 101;
const CHANNEL_B: i64 = 102;
const CHANNEL_C: i64 = 103;
const CONNECT: i64 = 1 << 20;
const VIEW_CHANNEL: i64 = 1 << 10;

async fn seed_preview_data(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2)",
        GUILD_ID,
        OWNER_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES
            ($1, $1, 1049600, '@everyone'),
            ($1, $2, $3, 'Listeners'),
            ($1, $4, $5, 'Admins')",
        GUILD_ID,
        VIEW_ROLE_ID,
        VIEW_CHANNEL,
        ADMIN_ROLE_ID,
        8, // ADMINISTRATOR
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES
            ($1, $2, 2, 'public'),
            ($3, $2, 2, 'restricted'),
            ($4, $2, 2, 'secret')",
        CHANNEL_A,
        GUILD_ID,
        CHANNEL_B,
        CHANNEL_C,
    )
    .execute(pool)
    .await?;
    // Listeners are denied CONNECT in the restricted channel (visible but not
    // joinable) and @everyone is denied VIEW_CHANNEL in the secret channel
    // (not visible at all).
    sqlx::query!(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $3), ($4, $5, 'role', 0, $6)",
        CHANNEL_B,
        VIEW_ROLE_ID,
        CONNECT,
        CHANNEL_C,
        GUILD_ID,
        VIEW_CHANNEL,
    )
    .execute(pool)
    .await?;
    // One finalized session in each channel so the tree distinguishes them.
    for (channel, user) in [(CHANNEL_A, 30i64), (CHANNEL_B, 40i64), (CHANNEL_C, 50i64)] {
        sqlx::query!(
            "INSERT INTO recording_sessions
                (guild_id, user_id, starting_channel_id, current_channel_id, state,
                 started_at, ended_at, end_reason, last_segment_index)
             VALUES ($1, $2, $3, $3, 'finalized',
                     to_timestamp(1), to_timestamp(3), 'test', 0)",
            GUILD_ID,
            user,
            channel,
        )
        .execute(pool)
        .await?;
    }
    sqlx::query!(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         VALUES
            ($1, $2, 'owner guild', NULL, true, 8, ARRAY[]::text[]),
            ($1, $3, 'allowed guild', NULL, false, 0, ARRAY[]::text[])",
        GUILD_ID,
        OWNER_ID,
        MEMBER_ID,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_view_shows_channels_role_can_see(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_preview_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_role_view),
            ),
    )
    .await;

    // Listeners: @everyone (1049600) + VIEW_CHANNEL, denied CONNECT on the
    // restricted channel. Both channels stay visible — CHANNEL_B only as a
    // channel they can see, not join.
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{VIEW_ROLE_ID}/channels"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["permission"], json!("1049600".to_string()));
    assert_eq!(body["can_manage_guild"], false);
    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    let by_id = |id: i64| -> Value {
        channels
            .iter()
            .find(|c| c["channel_id"] == json!(id.to_string()))
            .cloned()
            .expect("channel present")
    };
    assert_eq!(by_id(CHANNEL_A)["name"], "public");
    assert_eq!(by_id(CHANNEL_A)["can_join"], true);
    assert_eq!(by_id(CHANNEL_B)["name"], "restricted");
    assert_eq!(by_id(CHANNEL_B)["can_join"], false);

    // Admins: ADMINISTRATOR short-circuits to every voice channel, joinable.
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{GUILD_ID}/roles/{ADMIN_ROLE_ID}/channels"
        ))
        .insert_header(("Cookie", cookie))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["can_manage_guild"], true);
    let channel_ids: Vec<String> = body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["channel_id"].as_str().map(str::to_string))
        .collect();
    assert!(channel_ids.contains(&CHANNEL_A.to_string()));
    assert!(channel_ids.contains(&CHANNEL_B.to_string()));
    assert!(
        body["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["can_join"] == true)
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_preview_filters_recording_tree(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_preview_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_current_month_permission),
            ),
    )
    .await;

    // The owner sees every channel's sessions without impersonation, and no
    // access annotations are attached.
    let request = test::TestRequest::get()
        .uri(&format!("/api/current/{GUILD_ID}"))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    let channels = body.as_array().unwrap();
    let channel_ids: Vec<String> = channels
        .iter()
        .filter_map(|c| c["channel_id"].as_str().map(str::to_string))
        .collect();
    assert!(channel_ids.contains(&CHANNEL_A.to_string()));
    assert!(channel_ids.contains(&CHANNEL_B.to_string()));
    assert!(channel_ids.contains(&CHANNEL_C.to_string()));
    let first_session = first_session_of(channels).unwrap();
    assert!(first_session.get("access").is_none());

    // Impersonating Listeners keeps every session visible but annotated:
    // CHANNEL_A's session is playable, CHANNEL_B's is visible but not
    // joinable, CHANNEL_C's is not visible to the role at all.
    let request = test::TestRequest::get()
        .uri(&format!("/api/current/{GUILD_ID}?as_role={VIEW_ROLE_ID}"))
        .insert_header(("Cookie", cookie))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = test::read_body_json(response).await;
    let channels = body.as_array().unwrap();
    assert_eq!(
        session_access_of(channels, CHANNEL_A).unwrap(),
        "can-listen"
    );
    assert_eq!(
        session_access_of(channels, CHANNEL_B).unwrap(),
        "visible-only"
    );
    assert_eq!(session_access_of(channels, CHANNEL_C).unwrap(), "hidden");
    Ok(())
}

fn first_session_of(channels: &[Value]) -> Option<&Value> {
    channels.iter().find_map(|c| {
        c["dirs"].as_array()?.first().and_then(|dir| {
            dir["months"]
                .as_object()?
                .values()
                .next()
                .and_then(|files| files.as_array()?.first())
        })
    })
}

fn session_access_of(channels: &[Value], channel: i64) -> Option<String> {
    let files = channels
        .iter()
        .find(|c| c["channel_id"] == json!(channel.to_string()))
        .and_then(|c| c["dirs"].as_array()?.first())
        .and_then(|dir| dir["months"].as_object()?.values().next())
        .and_then(|files| files.as_array()?.first())?;
    files["access"].as_str().map(str::to_string)
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_preview_forbidden_for_plain_member(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_preview_data(&pool).await?;

    let cookie = access_cookie_for(MEMBER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_current_month_permission),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/current/{GUILD_ID}?as_role={VIEW_ROLE_ID}"))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_preview_unknown_role_returns_not_found(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_preview_data(&pool).await?;

    let cookie = access_cookie_for(OWNER_ID)?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_current_month_permission),
            ),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!("/api/current/{GUILD_ID}?as_role=999999"))
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

//! Voice presence and the member roster, served from the projections the bot
//! writes (`voice_presence`, `guild_members`, `guild_projection_state`).

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::members::{get_guild_roles, get_role_members, search_guild_members};
use web_server::presence::{get_voice_presence, voice_presence};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const GUILD_ID: i64 = 1;
const OWNER_ID: i64 = 5;
const MEMBER_ID: i64 = 10;
const MANAGER_ID: i64 = 11;
const ADMIN_ID: i64 = 12;
/// Everyone views and joins it.
const OPEN_CHANNEL: i64 = 100;
/// Everyone views it; nobody without a role may connect.
const LOOK_ONLY_CHANNEL: i64 = 200;
/// A stage channel only Insiders view.
const HIDDEN_STAGE: i64 = 300;
const INSIDER_ROLE_ID: i64 = 1001;
const MANAGER_ROLE_ID: i64 = 1002;
const ADMIN_ROLE_ID: i64 = 1003;
const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const MANAGE_GUILD: i64 = 1 << 5;
const ADMINISTRATOR: i64 = 1 << 3;
const CSRF: &str = "csrf-test-token";

fn cookie(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
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

/// A guild with three voice-type channels, three roles and members, and a
/// running bot that owns the projections, with someone in every channel.
async fn seed(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, $2)")
        .bind(GUILD_ID)
        .bind(OWNER_ID)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $1, $2, '@everyone'), ($1, $3, 0, 'Insiders'),
                ($1, $4, $5, 'Managers'), ($1, $6, $7, 'Admins')",
    )
    .bind(GUILD_ID)
    .bind(VIEW_CHANNEL | CONNECT)
    .bind(INSIDER_ROLE_ID)
    .bind(MANAGER_ROLE_ID)
    .bind(MANAGE_GUILD)
    .bind(ADMIN_ROLE_ID)
    .bind(ADMINISTRATOR)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $4, 2, 'Open'), ($2, $4, 2, 'Look only'), ($3, $4, 13, 'Hidden stage')",
    )
    .bind(OPEN_CHANNEL)
    .bind(LOOK_ONLY_CHANNEL)
    .bind(HIDDEN_STAGE)
    .bind(GUILD_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $3, 'role', 0, $4),
                ($2, $3, 'role', 0, $5),
                ($2, $6, 'role', $5, 0)",
    )
    .bind(LOOK_ONLY_CHANNEL)
    .bind(HIDDEN_STAGE)
    .bind(GUILD_ID)
    .bind(CONNECT)
    .bind(VIEW_CHANNEL)
    .bind(INSIDER_ROLE_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
         SELECT $1, user_id, 'guild', false, 0, ARRAY[]::text[]
           FROM UNNEST($2::bigint[]) AS user_id",
    )
    .bind(GUILD_ID)
    .bind(vec![MEMBER_ID, MANAGER_ID, ADMIN_ID])
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2), ($3, $4)")
        .bind(MANAGER_ID)
        .bind(MANAGER_ROLE_ID)
        .bind(ADMIN_ID)
        .bind(ADMIN_ROLE_ID)
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at)
         VALUES ('bot', 'active', 'active', now())",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO guild_projection_state
            (guild_id, owner_instance_id, generation, roster_complete_at, presence_synced_at)
         VALUES ($1, 'bot', 1, now(), now())",
    )
    .bind(GUILD_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, username, global_name, nickname, is_bot)
         VALUES ($1, 31, 'ann', NULL, NULL, false),
                ($1, 32, 'ben', 'Benjamin', NULL, false),
                ($1, 33, 'cat', 'Cat', 'Kitty', false),
                ($1, 34, 'sakiot', NULL, NULL, true),
                ($1, 35, 'under_score', NULL, NULL, false)",
    )
    .bind(GUILD_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO voice_presence (guild_id, user_id, channel_id, self_mute)
         VALUES ($1, 31, $2, false), ($1, 34, $2, false),
                ($1, 32, $3, true), ($1, 33, $4, false)",
    )
    .bind(GUILD_ID)
    .bind(OPEN_CHANNEL)
    .bind(LOOK_ONLY_CHANNEL)
    .bind(HIDDEN_STAGE)
    .execute(pool)
    .await?;
    Ok(())
}

/// Channel and member names per channel, as one viewer sees them.
async fn seen_by(
    pool: &PgPool,
    viewer: i64,
    as_role: Option<i64>,
) -> Result<(bool, Vec<(String, Vec<String>)>), Box<dyn std::error::Error>> {
    let presence = voice_presence(pool, GUILD_ID, viewer, as_role).await?;
    Ok((
        presence.available,
        presence
            .channels
            .into_iter()
            .map(|channel| {
                (
                    channel.name,
                    channel
                        .members
                        .into_iter()
                        .map(|member| member.name.unwrap_or_default())
                        .collect(),
                )
            })
            .collect(),
    ))
}

fn channels(items: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    items
        .iter()
        .map(|(channel, members)| {
            (
                channel.to_string(),
                members.iter().map(|member| member.to_string()).collect(),
            )
        })
        .collect()
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn members_see_presence_only_in_channels_they_can_view(pool: PgPool) -> TestResult {
    seed(&pool).await?;

    // VIEW_CHANNEL is enough (the look-only channel), and the hidden stage
    // channel is absent as if empty. Nicknames win over display names.
    assert_eq!(
        seen_by(&pool, MEMBER_ID, None).await?,
        (
            true,
            channels(&[("Look only", &["Benjamin"]), ("Open", &["ann", "sakiot"])])
        )
    );
    // An Administrator views every channel, stage channels included.
    assert_eq!(
        seen_by(&pool, ADMIN_ID, None).await?,
        (
            true,
            channels(&[
                ("Hidden stage", &["Kitty"]),
                ("Look only", &["Benjamin"]),
                ("Open", &["ann", "sakiot"]),
            ])
        )
    );
    // Moving into a hidden channel looks like leaving.
    sqlx::query("UPDATE voice_presence SET channel_id = $1 WHERE user_id = 31")
        .bind(HIDDEN_STAGE)
        .execute(&pool)
        .await?;
    assert_eq!(
        seen_by(&pool, MEMBER_ID, None).await?,
        (
            true,
            channels(&[("Look only", &["Benjamin"]), ("Open", &["sakiot"])])
        )
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_role_preview_of_presence_never_exceeds_the_managers_view(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    // Insiders could view the hidden stage, but a MANAGE_GUILD-only manager
    // cannot, so their preview leaves it out.
    assert_eq!(
        seen_by(&pool, MANAGER_ID, Some(INSIDER_ROLE_ID)).await?,
        (
            true,
            channels(&[("Look only", &["Benjamin"]), ("Open", &["ann", "sakiot"])])
        )
    );
    assert_eq!(
        seen_by(&pool, ADMIN_ID, Some(INSIDER_ROLE_ID)).await?,
        (
            true,
            channels(&[
                ("Hidden stage", &["Kitty"]),
                ("Look only", &["Benjamin"]),
                ("Open", &["ann", "sakiot"]),
            ])
        )
    );

    // Over HTTP, a member may not preview at all, and outsiders see nothing.
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_voice_presence),
            ),
    )
    .await;
    for (viewer, query, status) in [
        (
            MEMBER_ID,
            format!("?as_role={INSIDER_ROLE_ID}"),
            StatusCode::FORBIDDEN,
        ),
        (99, String::new(), StatusCode::FORBIDDEN),
        (MEMBER_ID, String::new(), StatusCode::OK),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/current/{GUILD_ID}/voice-presence{query}"))
                .insert_header(("Cookie", cookie(viewer)?))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status, "viewer {viewer} {query}");
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn presence_is_unknown_without_a_live_owner(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let unknown = (false, Vec::new());

    // The owner went silent (crashed): its rows no longer change.
    sqlx::query("UPDATE bot_instances SET heartbeat_at = now() - interval '5 minutes'")
        .execute(&pool)
        .await?;
    assert_eq!(seen_by(&pool, MEMBER_ID, None).await?, unknown);

    // It stopped and released the guild.
    sqlx::query("UPDATE bot_instances SET heartbeat_at = now(), state = 'stopped'")
        .execute(&pool)
        .await?;
    assert_eq!(seen_by(&pool, MEMBER_ID, None).await?, unknown);
    sqlx::query("UPDATE bot_instances SET state = 'active'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE guild_projection_state SET owner_instance_id = NULL")
        .execute(&pool)
        .await?;
    assert_eq!(seen_by(&pool, MEMBER_ID, None).await?, unknown);

    // First rollout: no bot has claimed the guild yet.
    sqlx::query("DELETE FROM guild_projection_state")
        .execute(&pool)
        .await?;
    assert_eq!(seen_by(&pool, MEMBER_ID, None).await?, unknown);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn managers_search_the_roster_and_everyone_counts_it(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(search_guild_members)
                    .service(get_guild_roles)
                    .service(get_role_members),
            ),
    )
    .await;
    let get = |viewer: i64, uri: String| {
        let app = &app;
        async move {
            let response = test::call_service(
                app,
                test::TestRequest::get()
                    .uri(&uri)
                    .insert_header(("Cookie", cookie(viewer)?))
                    .to_request(),
            )
            .await;
            let status = response.status();
            let body: Value = test::read_body_json(response).await;
            Ok::<_, Box<dyn std::error::Error>>((status, body))
        }
    };
    let names = |body: &Value| -> Vec<String> {
        body["members"]
            .as_array()
            .map(|members| {
                members
                    .iter()
                    .filter_map(|member| member["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let search = format!("/api/admin/guilds/{GUILD_ID}/members");

    // By name (nickname, display name or username), case-insensitively.
    let (status, body) = get(MANAGER_ID, format!("{search}?q=BEN")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&body), ["Benjamin"]);
    assert_eq!(body["complete"], true);
    let (_, body) = get(MANAGER_ID, format!("{search}?q=kit")).await?;
    assert_eq!(names(&body), ["Kitty"]);
    // By the start of an id.
    let (_, body) = get(MANAGER_ID, format!("{search}?q=3")).await?;
    assert_eq!(names(&body).len(), 5);
    let (_, body) = get(MANAGER_ID, format!("{search}?q=34")).await?;
    assert_eq!(names(&body), ["sakiot"]);
    assert_eq!(body["members"][0]["is_bot"], true);
    // Wildcards are literal.
    let (_, body) = get(MANAGER_ID, format!("{search}?q=_")).await?;
    assert_eq!(names(&body), ["under_score"]);

    // Bounded pages, by name.
    let (_, body) = get(MANAGER_ID, format!("{search}?limit=2")).await?;
    assert_eq!(names(&body), ["ann", "Benjamin"]);
    assert_eq!(body["next_offset"], 2);
    let (_, body) = get(MANAGER_ID, format!("{search}?limit=2&offset=4")).await?;
    assert_eq!(names(&body), ["under_score"]);
    assert_eq!(body["next_offset"], Value::Null);
    for bad in ["limit=0", "limit=51", "offset=-1", "offset=1001"] {
        let (status, _) = get(MANAGER_ID, format!("{search}?{bad}")).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = get(MEMBER_ID, search.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // @everyone counts and lists the roster.
    let (_, roles) = get(MANAGER_ID, format!("/api/admin/guilds/{GUILD_ID}/roles")).await?;
    let everyone = roles
        .as_array()
        .and_then(|roles| {
            roles
                .iter()
                .find(|role| role["role_id"] == json!(GUILD_ID.to_string()))
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(everyone["member_count"], 5);
    let (_, listed) = get(
        MANAGER_ID,
        format!("/api/admin/guilds/{GUILD_ID}/roles/{GUILD_ID}/members"),
    )
    .await?;
    assert_eq!(listed.as_array().map(Vec::len), Some(5));

    // A roster that was never complete reports so.
    sqlx::query("UPDATE guild_projection_state SET roster_complete_at = NULL")
        .execute(&pool)
        .await?;
    let (_, body) = get(MANAGER_ID, search).await?;
    assert_eq!(body["complete"], false);
    let (_, roles) = get(MANAGER_ID, format!("/api/admin/guilds/{GUILD_ID}/roles")).await?;
    let everyone = roles
        .as_array()
        .and_then(|roles| {
            roles
                .iter()
                .find(|role| role["role_id"] == json!(GUILD_ID.to_string()))
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(everyone["member_count"], Value::Null);
    Ok(())
}

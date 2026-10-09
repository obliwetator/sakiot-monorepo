//! Who belongs to a guild: the bot's roster for Discord logins, the seeded
//! `user_guilds` rows for dev logins (local, staging and preview builds).

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::config::Config;
use web_server::errors::AppError;
use web_server::permissions::{Permissions, Viewer, get_combined_perm_for_user};
use web_server::user::get_current_user_guilds;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const GUILD: i64 = 1;
/// A second guild the bot is in, with a complete roster the member is not on.
const OTHER_GUILD: i64 = 2;
const OWNER: i64 = 5;
const MEMBER: i64 = 10;
const DEV: i64 = 99;
const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const CSRF: &str = "csrf-test-token";

fn everyone() -> Permissions {
    Permissions::from_bits_retain(VIEW_CHANNEL | CONNECT)
}

fn dev(user_id: i64) -> Viewer {
    Viewer { user_id, dev: true }
}

/// The member is on the bot's complete roster of `GUILD`; the dev account has
/// only a seeded `user_guilds` row, as the seeding tools leave it.
async fn seed(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO guilds (id, owner_id, name, icon)
         VALUES ($1, $3, 'Zeta', 'zeta-icon'), ($2, $3, 'alpha', NULL)",
    )
    .bind(GUILD)
    .bind(OTHER_GUILD)
    .bind(OWNER)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1), ($2)")
        .bind(GUILD)
        .bind(OTHER_GUILD)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $1, $3, '@everyone'), ($2, $2, $3, '@everyone')",
    )
    .bind(GUILD)
    .bind(OTHER_GUILD)
    .bind(VIEW_CHANNEL | CONNECT)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
         VALUES ($1, $2, 'login snapshot', false, 0, ARRAY[]::text[]),
                ($1, $3, 'login snapshot', false, 0, ARRAY[]::text[])",
    )
    .bind(GUILD)
    .bind(MEMBER)
    .bind(DEV)
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
         VALUES ($1, 'bot', 1, now(), now()), ($2, 'bot', 1, now(), now())",
    )
    .bind(GUILD)
    .bind(OTHER_GUILD)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, username)
         VALUES ($1, $2, 'member'), ($1, $3, 'owner')",
    )
    .bind(GUILD)
    .bind(MEMBER)
    .bind(OWNER)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_complete_roster_stays_authoritative_after_the_bot_leaves(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let member = Viewer::discord(MEMBER);
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, member).await?,
        everyone()
    );

    // The bot was removed (or every instance stopped): the roster is no
    // longer kept current, but it is the last complete one and still decides.
    sqlx::query("DELETE FROM guilds_present")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE guild_projection_state SET owner_instance_id = NULL")
        .execute(&pool)
        .await?;
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, member).await?,
        everyone()
    );
    assert_eq!(
        get_combined_perm_for_user(&pool, OTHER_GUILD, member).await?,
        Permissions::empty()
    );

    // A guild the bot never tracked has no members at all, only its owner.
    sqlx::query("DELETE FROM guild_projection_state")
        .execute(&pool)
        .await?;
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, member).await?,
        Permissions::empty()
    );
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, Viewer::discord(OWNER)).await?,
        Permissions::all()
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn dev_logins_follow_the_seeded_guilds_without_a_roster(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    // The dev account is not on the roster, and staging's bot is in a
    // different server anyway; the seeded row makes it a member.
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, dev(DEV)).await?,
        everyone()
    );
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, Viewer::discord(DEV)).await?,
        Permissions::empty()
    );
    assert_eq!(
        get_combined_perm_for_user(&pool, OTHER_GUILD, dev(DEV)).await?,
        Permissions::empty()
    );

    // Without any roster the dev login is unaffected, while a Discord login
    // to a guild the bot is in cannot be answered yet.
    sqlx::query("DELETE FROM guild_projection_state")
        .execute(&pool)
        .await?;
    assert_eq!(
        get_combined_perm_for_user(&pool, GUILD, dev(DEV)).await?,
        everyone()
    );
    assert!(matches!(
        get_combined_perm_for_user(&pool, GUILD, Viewer::discord(MEMBER)).await,
        Err(AppError::MembershipUnavailable)
    ));
    Ok(())
}

fn config(dev_account_id: i64) -> Config {
    Config {
        database_url: String::new(),
        client_id: String::new(),
        client_secret: String::new(),
        access_secret: String::new(),
        refresh_secret: String::new(),
        dev_account_id,
        dev_login_secret: None,
        cors_allowed_origin: String::new(),
        oauth_allowed_opener_origins: Vec::new(),
        oauth_allowed_opener_host_suffixes: Vec::new(),
        discord_redirect_uri: String::new(),
        grpc_address: String::new(),
        fbi_agent_registry_secret: None,
        host: String::new(),
        port: 0,
        db_max_connections: 5,
        recording_permanent_delete_enabled: false,
        server_timing_header: false,
    }
}

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

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn the_guild_picker_lists_the_guilds_the_bot_counts_you_in(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(config(DEV)))
            .wrap(AuthMiddleware)
            .service(get_current_user_guilds),
    )
    .await;
    let picker = |user_id: i64| -> Result<_, Box<dyn std::error::Error>> {
        Ok(test::TestRequest::get()
            .uri("/users/current/guilds")
            .insert_header(("Cookie", cookie(user_id)?))
            .to_request())
    };

    // Live names and icons, ordered by name; permissions are calculated, not
    // the login snapshot's. The owner owns both guilds without being listed
    // on the second roster.
    let guilds: Value = test::call_and_read_body_json(&app, picker(OWNER)?).await;
    let all = (Permissions::all().bits()).to_string();
    assert_eq!(
        guilds,
        json!([
            {"id": "2", "name": "alpha", "icon": null, "owner": true, "permissions": all},
            {"id": "1", "name": "Zeta", "icon": "zeta-icon", "owner": true, "permissions": all},
        ])
    );
    let guilds: Value = test::call_and_read_body_json(&app, picker(MEMBER)?).await;
    assert_eq!(
        guilds,
        json!([{
            "id": "1", "name": "Zeta", "icon": "zeta-icon", "owner": false,
            "permissions": (VIEW_CHANNEL | CONNECT).to_string(),
        }])
    );
    // A Discord login of the dev account is not the dev login: no roster
    // entry, no guilds.
    let guilds: Value = test::call_and_read_body_json(&app, picker(DEV)?).await;
    assert_eq!(guilds, json!([]));

    // A guild whose roster is not complete is left out until it is; a guild
    // without a synced name shows its id.
    sqlx::query("UPDATE guild_projection_state SET roster_complete_at = NULL WHERE guild_id = $1")
        .bind(GUILD)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE guilds SET name = NULL WHERE id = $1")
        .bind(OTHER_GUILD)
        .execute(&pool)
        .await?;
    let response = test::call_service(&app, picker(MEMBER)?).await;
    assert_eq!(response.status(), StatusCode::OK);
    let guilds: Value = test::read_body_json(response).await;
    assert_eq!(guilds, json!([]));
    let guilds: Value = test::call_and_read_body_json(&app, picker(OWNER)?).await;
    assert_eq!(guilds[0]["name"], json!("2"));
    Ok(())
}

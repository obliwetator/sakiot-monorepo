mod support;

use actix_web::{App, http::Method, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::admin::keep_allowances::{get_keep_allowances, put_keep_allowance};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::keeping::{
    get_kept_recordings, get_removed_recordings, keep_recording, release_recording,
    release_recording_for_everyone,
};
use web_server::operators::{get_operator_guilds, is_operator, put_guild_limits};
use web_server::recording_deletion::{
    DeletionPolicy, delete_recording, enqueue_expired, restore_recording,
};

const GUILD_ID: i64 = 1;
const OWNER_ID: i64 = 20;
/// Holds the Moderator role: MANAGE_GUILD, so every management capability.
const MODERATOR_ID: i64 = 30;
const VIP_ID: i64 = 40;
const MEMBER_ID: i64 = 10;
/// Runs the bot; no member of the guild.
const OPERATOR_ID: i64 = 90;
const MODERATOR_ROLE_ID: i64 = 1001;
const VIP_ROLE_ID: i64 = 1002;
/// Open to everyone.
const LOUNGE_ID: i64 = 100;
/// Hidden from everyone but the owner.
const STAFF_ID: i64 = 200;
const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const MANAGE_GUILD: i64 = 1 << 5;
const CSRF: &str = "csrf-test-token";

type TestResult = Result<(), Box<dyn std::error::Error>>;

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
        "INSERT INTO discord_auth_user (id, username, avatar)
         SELECT id, 'user-' || id, '' FROM unnest($1::bigint[]) AS id",
        &[MEMBER_ID, MODERATOR_ID, VIP_ID, OWNER_ID, OPERATOR_ID],
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_names (user_id, username, global_name) VALUES ($1, 'member', 'Member')",
        MEMBER_ID
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO guild_recording_policy (guild_id, retention_days) VALUES ($1, 30)",
        GUILD_ID
    )
    .execute(pool)
    .await?;
    support::complete_rosters_from_user_guilds(pool).await?;
    Ok(())
}

/// A finalized session of `speaker` in `channel` that ended `days` ago.
async fn session(pool: &PgPool, speaker: i64, channel: i64, days: i32) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, state, started_at, ended_at)
         VALUES ($1, $2, $3, 'finalized',
                 now() - make_interval(days => $4 + 1), now() - make_interval(days => $4))
         RETURNING id",
        GUILD_ID,
        speaker,
        channel,
        days
    )
    .fetch_one(pool)
    .await
}

async fn clip(pool: &PgPool, clip_id: &str, session_id: i64, deleted: bool) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO clips (clip_id, guild_id, channel_id, channel_ids, speaker_ids, user_id,
                            start_time, original_file_name, saved_file_name,
                            recording_session_id, deleted_at)
         VALUES ($1, $2, $3, ARRAY[$3::bigint], ARRAY[$4::bigint], $4, 0, 'session', $1, $5,
                 CASE WHEN $6 THEN now() END)",
        clip_id,
        GUILD_ID,
        LOUNGE_ID,
        MEMBER_ID,
        session_id,
        deleted
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn removed(pool: &PgPool, session_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT deletion_requested_at IS NOT NULL AS "removed!"
             FROM recording_sessions WHERE id = $1"#,
        session_id
    )
    .fetch_one(pool)
    .await
}

fn ids(listing: &Value) -> Vec<String> {
    listing["recordings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|recording| recording["recording_session_id"].as_str())
        .map(str::to_owned)
        .collect()
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn members_keep_recordings_retention_removed_within_allowances_and_the_cap(
    pool: PgPool,
) -> TestResult {
    seed(&pool).await?;
    let mine = session(&pool, MEMBER_ID, LOUNGE_ID, 40).await?;
    let vips = session(&pool, VIP_ID, LOUNGE_ID, 40).await?;
    let staff = session(&pool, OWNER_ID, STAFF_ID, 40).await?;
    let recent = session(&pool, MEMBER_ID, LOUNGE_ID, 1).await?;
    clip(&pool, "cut-from-mine", mine, false).await?;
    clip(&pool, "deleted-before", mine, true).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_kept_recordings)
                    .service(get_removed_recordings)
                    .service(keep_recording)
                    .service(release_recording)
                    .service(release_recording_for_everyone)
                    .service(get_keep_allowances)
                    .service(put_keep_allowance)
                    .service(get_operator_guilds)
                    .service(put_guild_limits),
            ),
    )
    .await;
    let call = async |method: Method,
                      user_id: i64,
                      uri: String,
                      body: Option<Value>|
           -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let mut request = test::TestRequest::default()
            .method(method)
            .uri(&uri)
            .insert_header(("Cookie", access_cookie_for(user_id)?))
            .insert_header(("X-CSRF-Token", CSRF));
        if let Some(body) = body {
            request = request.set_json(body);
        }
        let response = test::call_service(&app, request.to_request()).await;
        let status = response.status();
        let body = test::read_body(response).await;
        Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    };
    let keep = async |user_id: i64, session_id: i64| {
        call(
            Method::PUT,
            user_id,
            format!("/api/users/current/guilds/{GUILD_ID}/kept-recordings/{session_id}"),
            None,
        )
        .await
    };
    let kept = async |user_id: i64| {
        call(
            Method::GET,
            user_id,
            format!("/api/guilds/{GUILD_ID}/kept-recordings"),
            None,
        )
        .await
    };
    let allow = async |user_id: i64, subject: Value, allowance: Value| {
        call(
            Method::PUT,
            user_id,
            format!("/api/admin/guilds/{GUILD_ID}/keep-allowances"),
            Some(json!({ "subject": subject, "allowance": allowance })),
        )
        .await
    };

    enqueue_expired(&pool).await?;
    assert!(removed(&pool, mine).await? && removed(&pool, vips).await?);
    assert!(removed(&pool, staff).await? && !removed(&pool, recent).await?);

    // What retention removed that the member may hear, and nothing else.
    let (status, listing) = call(
        Method::GET,
        MEMBER_ID,
        format!("/api/guilds/{GUILD_ID}/removed-recordings"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let mut listed = ids(&listing);
    listed.sort();
    let mut expected = vec![mine.to_string(), vips.to_string()];
    expected.sort();
    assert_eq!(listed, expected);
    let entry = &listing["recordings"][0];
    assert_eq!(entry["reason"], "retention");
    assert_eq!(entry["past_retention"], true);
    assert_eq!(entry["restorable"], false);
    assert_eq!(entry["channel_name"], "lounge");

    // Until an operator gives the guild a cap, nothing can be kept.
    let (status, refused) = keep(MEMBER_ID, mine).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        refused["message"],
        "This server may not keep recordings yet: the bot's operators decide how many."
    );
    let limits = format!("/api/operator/guilds/{GUILD_ID}/limits");
    let cap = |count: i32| Some(json!({ "kept_recordings": count }));
    for user_id in [OWNER_ID, MODERATOR_ID] {
        let (status, _) = call(Method::PUT, user_id, limits.clone(), cap(2)).await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert!(!is_operator(&pool, OPERATOR_ID).await?);
    sqlx::query!("INSERT INTO operators (user_id) VALUES ($1)", OPERATOR_ID)
        .execute(&pool)
        .await?;
    assert!(is_operator(&pool, OPERATOR_ID).await?);
    let (status, _) = call(Method::PUT, OPERATOR_ID, limits.clone(), cap(-1)).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(Method::PUT, OPERATOR_ID, limits.clone(), cap(2)).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, guilds) = call(
        Method::GET,
        OPERATOR_ID,
        "/api/operator/guilds".into(),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(guilds[0]["guild_id"], GUILD_ID.to_string());
    assert_eq!(guilds[0]["kept_recordings_cap"], 2);
    assert_eq!(guilds[0]["retention_days"], 30);
    let (status, _) = call(Method::GET, MEMBER_ID, "/api/operator/guilds".into(), None).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Holders of manage_settings share it out.
    let everyone = json!({ "kind": "everyone" });
    let vip_role = json!({ "kind": "role", "id": VIP_ROLE_ID.to_string() });
    let vip = json!({ "kind": "user", "id": VIP_ID.to_string() });
    assert_eq!(
        allow(MEMBER_ID, everyone.clone(), json!(5)).await?.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        allow(MODERATOR_ID, everyone.clone(), json!(1)).await?.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        allow(MODERATOR_ID, vip_role.clone(), json!(0)).await?.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        allow(
            MODERATOR_ID,
            json!({ "kind": "role", "id": "4242" }),
            json!(3)
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, allowances) = call(
        Method::GET,
        MODERATOR_ID,
        format!("/api/admin/guilds/{GUILD_ID}/keep-allowances"),
        None,
    )
    .await?;
    assert_eq!(allowances["cap"], 2);
    assert_eq!(allowances["allowances"][0]["subject"], everyone);
    assert_eq!(allowances["allowances"][1]["name"], "VIP");

    // Keeping brings the recording back, with the clips its removal hid
    // and only those.
    assert_eq!(keep(MEMBER_ID, mine).await?.0, StatusCode::NO_CONTENT);
    assert!(!removed(&pool, mine).await?);
    let visible =
        sqlx::query_scalar!("SELECT clip_id FROM clips WHERE deleted_at IS NULL ORDER BY clip_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(visible, ["cut-from-mine"]);
    let restoration = sqlx::query!(
        "SELECT reason, restored_by, kept FROM recording_restorations WHERE recording_session_id = $1",
        mine
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        (
            restoration.reason.as_str(),
            restoration.restored_by,
            restoration.kept
        ),
        ("retention", MEMBER_ID, true)
    );
    // Keeping it again changes nothing; another is past the allowance.
    assert_eq!(keep(MEMBER_ID, mine).await?.0, StatusCode::NO_CONTENT);
    let (status, refused) = keep(MEMBER_ID, vips).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        refused["message"],
        "You already keep 1 recordings, all you may keep in this server."
    );
    // Beyond their hearing, it is not there to keep.
    assert_eq!(keep(MEMBER_ID, staff).await?.0, StatusCode::NOT_FOUND);

    let (_, page) = kept(MEMBER_ID).await?;
    assert_eq!(
        (
            &page["cap"],
            &page["kept"],
            &page["allowance"],
            &page["used"]
        ),
        (&json!(2), &json!(1), &json!(1), &json!(1))
    );
    assert_eq!(ids(&page), [mine.to_string()]);
    assert_eq!(page["recordings"][0]["mine"], true);
    assert_eq!(page["recordings"][0]["speaker_name"], "Member");

    // Retention leaves a kept recording be.
    enqueue_expired(&pool).await?;
    assert!(!removed(&pool, mine).await?);

    // The owner hears the staff channel; that fills the cap.
    assert_eq!(keep(OWNER_ID, staff).await?.0, StatusCode::NO_CONTENT);
    // The member sees what they may hear, not the owner's staff recording.
    let (_, page) = kept(MEMBER_ID).await?;
    assert_eq!(page["kept"], 2);
    assert_eq!(ids(&page), [mine.to_string()]);

    // The VIP role may keep none, but the VIP's own allowance wins; and a
    // recording someone else keeps takes no more of the full cap.
    assert_eq!(keep(VIP_ID, mine).await?.0, StatusCode::CONFLICT);
    assert_eq!(
        allow(MODERATOR_ID, vip.clone(), json!(3)).await?.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(keep(VIP_ID, mine).await?.0, StatusCode::NO_CONTENT);
    let (_, page) = kept(VIP_ID).await?;
    assert_eq!((&page["kept"], &page["used"]), (&json!(2), &json!(1)));
    assert_eq!(page["recordings"][0]["keepers"], 2);
    let (status, refused) = keep(VIP_ID, vips).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        refused["message"],
        "This server already keeps all 2 recordings it may keep."
    );

    // Letting go: the member's keep, then everyone's, by a manager.
    let release = format!("/api/users/current/guilds/{GUILD_ID}/kept-recordings/{mine}");
    assert_eq!(
        call(Method::DELETE, MEMBER_ID, release, None).await?.0,
        StatusCode::NO_CONTENT
    );
    enqueue_expired(&pool).await?;
    assert!(!removed(&pool, mine).await?, "the VIP still keeps it");
    let release_all = format!("/api/admin/guilds/{GUILD_ID}/kept-recordings/{mine}");
    assert_eq!(
        call(Method::DELETE, MEMBER_ID, release_all.clone(), None)
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(Method::DELETE, MODERATOR_ID, release_all, None)
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    enqueue_expired(&pool).await?;
    assert!(removed(&pool, mine).await?);
    // Removed again, its clip went with it.
    let visible: i64 =
        sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM clips WHERE deleted_at IS NULL"#)
            .fetch_one(&pool)
            .await?;
    assert_eq!(visible, 0);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn managers_restore_their_removals_within_retention_and_keep_older_ones(
    pool: PgPool,
) -> TestResult {
    seed(&pool).await?;
    sqlx::query!(
        "INSERT INTO guild_limits (guild_id, kept_recordings, updated_by) VALUES ($1, 5, $2)",
        GUILD_ID,
        OPERATOR_ID
    )
    .execute(&pool)
    .await?;
    let recent = session(&pool, MEMBER_ID, LOUNGE_ID, 1).await?;
    let old = session(&pool, MEMBER_ID, LOUNGE_ID, 40).await?;
    sqlx::query!(
        "INSERT INTO kept_recordings (recording_session_id, user_id, guild_id) VALUES ($1, $2, $3)",
        old,
        MEMBER_ID,
        GUILD_ID
    )
    .execute(&pool)
    .await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(DeletionPolicy::default()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(delete_recording)
                    .service(restore_recording)
                    .service(get_removed_recordings)
                    .service(keep_recording),
            ),
    )
    .await;
    let call = async |method: Method,
                      user_id: i64,
                      uri: String|
           -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
        let request = test::TestRequest::default()
            .method(method)
            .uri(&uri)
            .insert_header(("Cookie", access_cookie_for(user_id)?))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request();
        let response = test::call_service(&app, request).await;
        let status = response.status();
        let body = test::read_body(response).await;
        Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    };
    let remove = |session_id: i64| format!("/api/admin/guilds/{GUILD_ID}/recordings/{session_id}");
    let restore =
        |session_id: i64| format!("/api/admin/guilds/{GUILD_ID}/recordings/{session_id}/restore");
    let keep = |session_id: i64| {
        format!("/api/users/current/guilds/{GUILD_ID}/kept-recordings/{session_id}")
    };
    let listing = format!("/api/guilds/{GUILD_ID}/removed-recordings");

    for session_id in [recent, old] {
        assert_eq!(
            call(Method::DELETE, MODERATOR_ID, remove(session_id))
                .await?
                .0,
            StatusCode::ACCEPTED
        );
    }
    // Removing a kept recording frees its keep.
    let keeps: i64 = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM kept_recordings"#)
        .fetch_one(&pool)
        .await?;
    assert_eq!(keeps, 0);

    // A manager's removals are for managers to see and undo.
    let (_, members) = call(Method::GET, MEMBER_ID, listing.clone()).await?;
    assert!(ids(&members).is_empty());
    assert_eq!(
        call(Method::PUT, MEMBER_ID, keep(recent)).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(Method::POST, MEMBER_ID, restore(recent)).await?.0,
        StatusCode::FORBIDDEN
    );
    let (_, managers) = call(Method::GET, MODERATOR_ID, listing.clone()).await?;
    assert_eq!(ids(&managers), [old.to_string(), recent.to_string()]);
    let newest = &managers["recordings"][1];
    assert_eq!(newest["reason"], "manager");
    assert_eq!(newest["removed_by"], MODERATOR_ID.to_string());
    assert_eq!(newest["restorable"], true);
    assert_eq!(managers["recordings"][0]["restorable"], false);

    assert_eq!(
        call(Method::POST, MODERATOR_ID, restore(recent)).await?.0,
        StatusCode::NO_CONTENT
    );
    assert!(!removed(&pool, recent).await?);
    // Restored once, there is nothing left to restore.
    assert_eq!(
        call(Method::POST, MODERATOR_ID, restore(recent)).await?.0,
        StatusCode::NOT_FOUND
    );

    // Past retention, only keeping brings it back.
    let (status, refused) = call(Method::POST, MODERATOR_ID, restore(old)).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        refused["message"],
        "This recording is older than the retention period: keep it to bring it back."
    );
    assert_eq!(
        call(Method::PUT, MODERATOR_ID, keep(old)).await?.0,
        StatusCode::NO_CONTENT
    );
    assert!(!removed(&pool, old).await?);
    let (_, managers) = call(Method::GET, MODERATOR_ID, listing).await?;
    assert!(ids(&managers).is_empty());
    Ok(())
}

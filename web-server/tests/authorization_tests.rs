use std::collections::HashMap;

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::RwLock;
use web_server::admin::voice_settings::{
    delete_voice_settings, get_voice_settings, put_voice_settings,
};
use web_server::audio::{
    LiveContainer, SilenceJobContainer, WaveformProgressContainer, create_session_clip,
    download_audio, download_session, get_audio, get_recording_events, get_session_events,
    get_session_manifest, get_session_segment, get_session_waveform, get_waveform_data,
    live_playlist, live_segment, live_state, rebuild_session_waveform, remove_session_silence,
    remove_silence, session_live_playlist, session_live_segment,
};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::clips::{create_clip, delete as delete_clip, get_clip, get_clips};
use web_server::permissions::visible_channels_for_user;
use web_server::stamps::get_stamps;

const USER_ID: i64 = 10;
const OTHER_USER_ID: i64 = 20;
const ALLOWED_GUILD_ID: i64 = 1;
const FORBIDDEN_GUILD_ID: i64 = 2;
const ALLOWED_CHANNEL_ID: i64 = 100;
const FORBIDDEN_CHANNEL_ID: i64 = 200;
const CONNECT_PERMISSION: i64 = 1 << 20;
const VIEW_CHANNEL_PERMISSION: i64 = 1 << 10;
const BASE_VOICE_PERMISSIONS: i64 = CONNECT_PERMISSION | VIEW_CHANNEL_PERMISSION;
const CSRF: &str = "csrf-test-token";

fn access_cookie_value() -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        USER_ID,
        AuthKind::Discord,
        CSRF.to_string(),
        &EncodingKey::from_secret(b"test_secret"),
    )?;
    Ok(format!("{ACCESS_TOKEN_COOKIE}={token}"))
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn one_inaccessible_fragment_denies_every_session_endpoint(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    // One denied fragment must fail the session atomically: never return a
    // permitted subset of its audio, metadata, or derived media.
    seed_authorization_data(&pool).await?;
    let denied_channel_id = ALLOWED_CHANNEL_ID + 1;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'denied-in-session')",
    )
    .bind(denied_channel_id)
    .bind(ALLOWED_GUILD_ID)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions
            (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'user', 0, $3)",
    )
    .bind(denied_channel_id)
    .bind(USER_ID)
    .bind(CONNECT_PERMISSION)
    .execute(&pool)
    .await?;

    let session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES ($1, $2, $3, $4, 'finalized',
                 to_timestamp(1), to_timestamp(3), 'test', 1)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(OTHER_USER_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(denied_channel_id)
    .fetch_one(&pool)
    .await?;
    let first_audio_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('session-allowed-fragment', $1, $2, $3, 1970, 1,
                 1000, 2000, $4, 0)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('session-denied-fragment', $1, $2, $3, 1970, 1,
                 2000, 3000, $4, 1)",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(denied_channel_id)
    .bind(OTHER_USER_ID)
    .bind(session_id)
    .execute(&pool)
    .await?;

    let cookie = access_cookie_value()?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(
                web_server::media_archive::MediaArchive::disabled(),
            ))
            .app_data(web::Data::new(WaveformProgressContainer(RwLock::new(
                HashMap::new(),
            ))))
            .app_data(web::Data::new(LiveContainer::default()))
            .app_data(web::Data::new(SilenceJobContainer::default()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_session_manifest)
                    .service(get_session_events)
                    .service(get_session_waveform)
                    .service(rebuild_session_waveform)
                    .service(download_session)
                    .service(create_session_clip)
                    .service(remove_session_silence)
                    .service(session_live_playlist)
                    .service(session_live_segment)
                    .service(get_session_segment)
                    .service(get_audio)
                    .service(download_audio)
                    .service(get_waveform_data)
                    .service(get_recording_events)
                    .service(remove_silence)
                    .service(live_playlist)
                    .service(live_state)
                    .service(live_segment)
                    .service(create_clip),
            ),
    )
    .await;

    let forbidden_gets = [
        format!("/api/audio/sessions/{session_id}/manifest"),
        format!("/api/audio/sessions/{session_id}/events"),
        format!("/api/audio/sessions/{session_id}/waveform"),
        format!("/api/audio/sessions/{session_id}/download"),
        format!("/api/audio/sessions/{session_id}/segments/{first_audio_id}"),
        format!("/api/audio/sessions/{session_id}/live/{first_audio_id}/playlist.m3u8"),
        format!("/api/audio/sessions/{session_id}/live/{first_audio_id}/seg_00000.m4s"),
        format!(
            "/api/audio/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
        format!(
            "/api/download/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
        format!(
            "/api/audio/waveform/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
        format!(
            "/api/audio/events/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
        format!(
            "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment/playlist.m3u8"
        ),
        format!(
            "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment/state"
        ),
        format!(
            "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment/seg_00000.m4s"
        ),
    ];
    for uri in forbidden_gets {
        let request = test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }

    for uri in [
        format!("/api/audio/sessions/{session_id}/waveform/rebuild"),
        format!("/api/audio/sessions/{session_id}/clips"),
        format!("/api/audio/sessions/{session_id}/remove-silence"),
        format!(
            "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
        format!(
            "/api/remove_silence/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
        ),
    ] {
        let request = test::TestRequest::post()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("X-CSRF-Token", CSRF))
            .insert_header(("Idempotency-Key", "session-auth-test"))
            .set_json(json!({"start": 0.0, "end": 1.0}))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn voice_settings_require_manager_and_restore_default(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    // Keep an OAuth-era manager grant in the database. Authorization must use
    // the live role cache below, including after that role permission is revoked.
    sqlx::query("UPDATE user_guilds SET permissions = $1 WHERE id = $2 AND user_id = $3")
        .bind(1_i64 << 5)
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE roles SET permission = $1 WHERE role_id = $2")
        .bind(BASE_VOICE_PERMISSIONS | (1_i64 << 5))
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;
    let cookie = access_cookie_value()?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(
                web_server::media_archive::MediaArchive::disabled(),
            ))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_voice_settings)
                    .service(put_voice_settings)
                    .service(delete_voice_settings),
            ),
    )
    .await;
    let uri = format!("/api/admin/guilds/{ALLOWED_GUILD_ID}/voice-settings");

    let request = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response: serde_json::Value = test::call_and_read_body_json(&app, request).await;
    assert_eq!(response["pending_cap_seconds"], 21_600);
    assert_eq!(response["is_default"], true);

    let request = test::TestRequest::put()
        .uri(&uri)
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"pending_cap_seconds": 59}))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::BAD_REQUEST
    );

    let request = test::TestRequest::put()
        .uri(&uri)
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"pending_cap_seconds": 120}))
        .to_request();
    let response: serde_json::Value = test::call_and_read_body_json(&app, request).await;
    assert_eq!(response["pending_cap_seconds"], 120);
    assert_eq!(response["is_default"], false);

    let request = test::TestRequest::delete()
        .uri(&uri)
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .to_request();
    let response: serde_json::Value = test::call_and_read_body_json(&app, request).await;
    assert_eq!(response["pending_cap_seconds"], 21_600);
    assert_eq!(response["is_default"], true);
    let overrides: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_voice_settings WHERE guild_id = $1")
            .bind(ALLOWED_GUILD_ID)
            .fetch_one(&pool)
            .await?;
    assert_eq!(overrides, 0);

    sqlx::query("UPDATE roles SET permission = $1 WHERE role_id = $2")
        .bind(BASE_VOICE_PERMISSIONS)
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;
    let request = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Cookie", cookie))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}

fn access_keys() -> AccessKeys {
    AccessKeys {
        access_encode: EncodingKey::from_secret(b"test_secret"),
        refresh_encode: EncodingKey::from_secret(b"test_secret"),
        access_decode: DecodingKey::from_secret(b"test_secret"),
        refresh_decode: DecodingKey::from_secret(b"test_secret"),
    }
}

async fn seed_authorization_data(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query!("INSERT INTO channel_type (id, type) VALUES (2, 'voice')")
        .execute(pool)
        .await?;
    sqlx::query!(
        "INSERT INTO guilds (id, owner_id) VALUES ($1, $2), ($3, $2)",
        ALLOWED_GUILD_ID,
        OTHER_USER_ID,
        FORBIDDEN_GUILD_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $1, $3, '@everyone'), ($2, $2, $3, '@everyone')",
        ALLOWED_GUILD_ID,
        FORBIDDEN_GUILD_ID,
        BASE_VOICE_PERMISSIONS,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'allowed'), ($3, $4, 2, 'forbidden')",
        ALLOWED_CHANNEL_ID,
        ALLOWED_GUILD_ID,
        FORBIDDEN_CHANNEL_ID,
        FORBIDDEN_GUILD_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         VALUES ($1, $2, 'allowed guild', NULL, false, 0, ARRAY[]::text[])",
        ALLOWED_GUILD_ID,
        USER_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO audio_files (file_name, guild_id, channel_id, user_id, year, month)
         VALUES ('forbidden-rec', $1, $2, $3, 2026, 5)",
        FORBIDDEN_GUILD_ID,
        FORBIDDEN_CHANNEL_ID,
        OTHER_USER_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, user_id, saved_file_name, start_time)
         VALUES
            ('forbidden-clip', $1, $2, $3, '2026/05/forbidden.ogg', 0),
            ('own-clip', $4, $5, $6, '2026/05/own.ogg', 0)",
        FORBIDDEN_GUILD_ID,
        FORBIDDEN_CHANNEL_ID,
        OTHER_USER_ID,
        ALLOWED_GUILD_ID,
        ALLOWED_CHANNEL_ID,
        USER_ID,
    )
    .execute(pool)
    .await?;
    sqlx::query!(
        "INSERT INTO stamps (guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts)
         VALUES ($1, $2, $3, $3, 1000)",
        FORBIDDEN_GUILD_ID,
        FORBIDDEN_CHANNEL_ID,
        OTHER_USER_ID,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn view_channel_deny_hides_voice_channel_with_inherited_connect(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let hidden_channel_id = ALLOWED_CHANNEL_ID + 1;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'hidden')",
    )
    .bind(hidden_channel_id)
    .bind(ALLOWED_GUILD_ID)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $3)",
    )
    .bind(hidden_channel_id)
    .bind(ALLOWED_GUILD_ID)
    .bind(VIEW_CHANNEL_PERMISSION)
    .execute(&pool)
    .await?;

    let pool = web::Data::new(pool);
    let visible = visible_channels_for_user(&pool, ALLOWED_GUILD_ID, USER_ID).await?;
    assert!(visible.contains(&ALLOWED_CHANNEL_ID));
    assert!(!visible.contains(&hidden_channel_id));

    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn stamps_include_only_fully_accessible_logical_sessions(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let denied_channel_id = ALLOWED_CHANNEL_ID + 1;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'denied-in-session')",
    )
    .bind(denied_channel_id)
    .bind(ALLOWED_GUILD_ID)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions
            (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'user', 0, $3)",
    )
    .bind(denied_channel_id)
    .bind(USER_ID)
    .bind(CONNECT_PERMISSION)
    .execute(&pool)
    .await?;

    let accessible_session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES ($1, $2, $3, $3, 'finalized',
                 to_timestamp(1), to_timestamp(3), 'test', 1)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(OTHER_USER_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('accessible-first', $1, $2, $3, 1970, 1,
                 1000, 2000, $4, 0)",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(accessible_session_id)
    .execute(&pool)
    .await?;
    let accessible_audio_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('accessible-second', $1, $2, $3, 1970, 1,
                 2000, 3000, $4, 1)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(accessible_session_id)
    .fetch_one(&pool)
    .await?;

    let restricted_session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES ($1, $2, $3, $4, 'finalized',
                 to_timestamp(4), to_timestamp(6), 'test', 1)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(OTHER_USER_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(denied_channel_id)
    .fetch_one(&pool)
    .await?;
    let restricted_audio_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('restricted-allowed-fragment', $1, $2, $3, 1970, 1,
                 4000, 5000, $4, 0)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(restricted_session_id)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('restricted-denied-fragment', $1, $2, $3, 1970, 1,
                 5000, 6000, $4, 1)",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(denied_channel_id)
    .bind(OTHER_USER_ID)
    .bind(restricted_session_id)
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO stamps
            (guild_id, channel_id, target_user_id, stamper_user_id,
             stamp_ts, offset_ms, audio_file_id, recording_session_id, note)
         VALUES
            ($1, $2, $3, $3, 2500, 100, $4, $5, 'accessible-session'),
            ($1, $2, $3, $3, 4500, 0, $6, $7, 'restricted-session'),
            ($1, $2, $3, $3, 2600, 0, NULL, $5, 'session-without-fragment')",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(accessible_audio_id)
    .bind(accessible_session_id)
    .bind(restricted_audio_id)
    .bind(restricted_session_id)
    .execute(&pool)
    .await?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .app_data(web::Data::new(access_keys()))
            .service(web::scope("/api").wrap(AuthMiddleware).service(get_stamps)),
    )
    .await;
    let request = test::TestRequest::get()
        .uri("/api/stamps/1")
        .insert_header(("Cookie", access_cookie_value()?))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Vec<serde_json::Value> = test::read_body_json(response).await;

    let accessible = body
        .iter()
        .find(|stamp| stamp["note"] == "accessible-session")
        .expect("accessible stamp");
    assert_eq!(
        accessible["recording_session_id"],
        accessible_session_id.to_string()
    );
    assert_eq!(accessible["segment_index"], 1);
    assert_eq!(accessible["session_started_at_ms"], 1000);
    assert_eq!(accessible["session_fragment_count"], 2);

    let without_fragment = body
        .iter()
        .find(|stamp| stamp["note"] == "session-without-fragment")
        .expect("session-only stamp");
    assert_eq!(
        without_fragment["recording_session_id"],
        accessible_session_id.to_string()
    );
    assert!(without_fragment["audio_file_id"].is_null());
    assert!(without_fragment["segment_index"].is_null());
    assert_eq!(without_fragment["session_started_at_ms"], 1000);
    assert_eq!(without_fragment["session_fragment_count"], 2);

    let restricted = body
        .iter()
        .find(|stamp| stamp["note"] == "restricted-session")
        .expect("restricted stamp");
    assert!(restricted["recording_session_id"].is_null());
    assert!(restricted["session_started_at_ms"].is_null());
    assert!(restricted["session_fragment_count"].is_null());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn forbidden_cross_guild_requests_are_rejected(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let cookie = access_cookie_value()?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(
                web_server::media_archive::MediaArchive::disabled(),
            ))
            .app_data(web::Data::new(SilenceJobContainer::default()))
            .app_data(web::Data::new(WaveformProgressContainer(RwLock::new(
                HashMap::new(),
            ))))
            .app_data(web::Data::new(LiveContainer::default()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_audio)
                    .service(download_audio)
                    .service(get_waveform_data)
                    .service(remove_silence)
                    .service(live_playlist)
                    .service(live_state)
                    .service(live_segment)
                    .service(get_clips)
                    .service(get_clip)
                    .service(create_clip)
                    .service(delete_clip)
                    .service(get_stamps),
            ),
    )
    .await;

    let forbidden_gets = [
        "/api/audio/2/200/2026/5/forbidden-rec.ogg",
        "/api/download/2/200/2026/5/forbidden-rec.ogg",
        "/api/audio/waveform/2/200/2026/5/forbidden-rec",
        "/api/audio/live/2/200/2026/5/forbidden-rec/playlist.m3u8",
        "/api/audio/live/2/200/2026/5/forbidden-rec/state",
        "/api/audio/live/2/200/2026/5/forbidden-rec/seg_00000.m4s",
        "/api/audio/clips/2",
        "/api/audio/clips/2/forbidden-clip",
        "/api/stamps/2",
    ];

    for uri in forbidden_gets {
        let req = test::TestRequest::get()
            .uri(uri)
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("Idempotency-Key", "forbidden-test"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{uri}");
    }

    let req = test::TestRequest::post()
        .uri("/api/remove_silence/2/200/2026/5/forbidden-rec")
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .insert_header(("Idempotency-Key", "forbidden-test"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let req = test::TestRequest::post()
        .uri("/api/audio/clips/create/2/200/2026/5/forbidden-rec")
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"start": 0.0, "end": 2.0, "name": "forbidden"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let req = test::TestRequest::delete()
        .uri("/api/audio/clips/2/forbidden-clip")
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let req = test::TestRequest::delete()
        .uri("/api/audio/clips/1/own-clip")
        .insert_header(("Cookie", cookie))
        .insert_header(("X-CSRF-Token", CSRF))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let row = sqlx::query!(
        "SELECT deleted_at FROM clips WHERE guild_id = $1 AND clip_id = 'own-clip'",
        ALLOWED_GUILD_ID
    )
    .fetch_one(&pool)
    .await?;
    assert!(row.deleted_at.is_some());

    Ok(())
}

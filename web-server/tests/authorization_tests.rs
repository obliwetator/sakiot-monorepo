use std::collections::HashMap;

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::RwLock;
use web_server::admin::cooldowns::{
    delete_user_override, get_guild_cooldown, list_user_overrides, set_guild_cooldown,
    set_user_override,
};
use web_server::admin::voice_settings::{
    delete_voice_settings, get_voice_settings, put_voice_settings,
};
use web_server::audio::{
    LiveContainer, SessionMixContainer, SilenceJobContainer, WaveformProgressContainer,
    create_session_clip, download_audio, download_session, generate_session_channel_mix, get_audio,
    get_recording_events, get_session_channel_mix, get_session_channel_mix_media,
    get_session_events, get_session_manifest, get_session_segment, get_session_silence_free,
    get_session_silence_free_waveform, get_session_silence_removal_status, get_session_waveform,
    get_waveform_data, live_playlist, live_segment, live_state,
    rebuild_session_silence_free_waveform, rebuild_session_waveform, remove_session_silence,
    remove_silence, session_live_playlist, session_live_segment,
};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::clips::{create_clip, delete as delete_clip, get_clip, get_clips, rename_clip};
use web_server::permissions::{Permissions, get_combined_perm_for_user, visible_channels_for_user};
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
    access_cookie_for(USER_ID)
}

fn access_cookie_for(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
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
            .app_data(web::Data::new(SessionMixContainer::default()))
            .app_data(web::Data::new(SilenceJobContainer::default()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_session_manifest)
                    .service(get_session_events)
                    .service(get_session_channel_mix)
                    .service(generate_session_channel_mix)
                    .service(get_session_channel_mix_media)
                    .service(get_session_waveform)
                    .service(rebuild_session_waveform)
                    .service(download_session)
                    .service(create_session_clip)
                    .service(get_session_silence_removal_status)
                    .service(remove_session_silence)
                    .service(get_session_silence_free)
                    .service(get_session_silence_free_waveform)
                    .service(rebuild_session_silence_free_waveform)
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
        format!("/api/audio/sessions/{session_id}/channel-mix"),
        format!("/api/audio/sessions/{session_id}/channel-mix/media"),
        format!("/api/audio/sessions/{session_id}/silence-free"),
        format!("/api/audio/sessions/{session_id}/silence-free/waveform"),
        format!("/api/audio/sessions/{session_id}/remove-silence"),
        format!("/api/audio/sessions/{session_id}/channel-mix"),
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
        format!("/api/audio/sessions/{session_id}/silence-free/waveform/rebuild"),
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
async fn oauth_owner_snapshot_grants_full_permissions(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    // The local tooling grants the dev account `owner = true` in user_guilds;
    // the agent keeps that flag fresh (sync_guild_owner) and drops the row on
    // member removal, so the snapshot is trusted for owner access even when
    // `guilds.owner_id` names someone else (e.g. an imported guild).
    sqlx::query("UPDATE user_guilds SET owner = true WHERE id = $1 AND user_id = $2")
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .execute(&pool)
        .await?;

    let pool = web::Data::new(pool);
    let granted = get_combined_perm_for_user(&pool, ALLOWED_GUILD_ID, USER_ID).await?;
    assert_eq!(granted, Permissions::all());

    // A plain membership snapshot (owner = false) without roles stays at
    // @everyone's level: only the flag, not the row, grants owner powers.
    sqlx::query("UPDATE user_guilds SET owner = false WHERE id = $1 AND user_id = $2")
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .execute(pool.get_ref())
        .await?;
    let demoted = get_combined_perm_for_user(&pool, ALLOWED_GUILD_ID, USER_ID).await?;
    assert!(!demoted.contains(Permissions::ADMINISTRATOR));

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
                    .service(create_clip)
                    .service(rename_clip)
                    .service(get_clip)
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

    let req = test::TestRequest::put()
        .uri("/api/audio/clips/2/forbidden-clip")
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"name": "not allowed"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let req = test::TestRequest::put()
        .uri("/api/audio/clips/1/own-clip")
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"name": "  renamed clip  "}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let name = sqlx::query_scalar::<_, Option<String>>(
        "SELECT name FROM clips WHERE guild_id = $1 AND clip_id = 'own-clip'",
    )
    .bind(ALLOWED_GUILD_ID)
    .fetch_one(&pool)
    .await?;
    assert_eq!(name.as_deref(), Some("renamed clip"));

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

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn clip_list_is_ordered_by_name_case_insensitively(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    // Inserted out of order, and with mixed case, so this fails if the endpoint
    // ever falls back to heap order. seed_authorization_data's own clip has a
    // null name, which must sort last.
    sqlx::query(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, user_id, saved_file_name, start_time, name)
         VALUES
            ('clip-zeta', $1, $2, $3, '2026/05/zeta.ogg', 0, 'zeta'),
            ('clip-alpha', $1, $2, $3, '2026/05/alpha.ogg', 0, 'Alpha'),
            ('clip-beta', $1, $2, $3, '2026/05/beta.ogg', 0, 'beta'),
            ('clip-alpha-two', $1, $2, $3, '2026/05/alpha2.ogg', 0, 'alpha 2')",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(USER_ID)
    .execute(&pool)
    .await?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(web::scope("/api").wrap(AuthMiddleware).service(get_clips)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri(&format!("/api/audio/clips/{ALLOWED_GUILD_ID}"))
        .insert_header(("Cookie", access_cookie_value()?))
        .to_request();
    let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let names: Vec<Option<&str>> = body
        .as_array()
        .expect("clip list is an array")
        .iter()
        .map(|clip| clip["name"].as_str())
        .collect();

    assert_eq!(
        names,
        vec![
            Some("Alpha"),
            Some("alpha 2"),
            Some("beta"),
            Some("zeta"),
            None
        ],
        "clips must come back in case-insensitive name order, unnamed last"
    );
    Ok(())
}

/// An unclippable range must be refused with 400 and must not leave a clip row
/// behind. These ranges are rejected before any media work, so the recording
/// need not exist on disk.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn create_clip_rejects_invalid_ranges_without_storing_a_clip(
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
            .app_data(web::Data::new(WaveformProgressContainer(RwLock::new(
                HashMap::new(),
            ))))
            .service(web::scope("/api").wrap(AuthMiddleware).service(create_clip)),
    )
    .await;
    let uri = format!(
        "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/2026/5/p0-1-range-check"
    );
    let clips_before: i64 = sqlx::query_scalar("SELECT count(*) FROM clips")
        .fetch_one(&pool)
        .await?;

    for range in [
        json!({"start": -5.0, "end": 1.0}),
        json!({"start": 0.0, "end": 0.0}),
        json!({"start": 0.0, "end": 0.5}),
        json!({"start": 0.0, "end": 25.0}),
        json!({"start": 2.0, "end": 1.0}),
    ] {
        let request = test::TestRequest::post()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("X-CSRF-Token", CSRF))
            .insert_header(("Idempotency-Key", "clip-range-test"))
            .set_json(range.clone())
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{range}");
    }

    let clips_after: i64 = sqlx::query_scalar("SELECT count(*) FROM clips")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        clips_before, clips_after,
        "rejected ranges must not insert clip rows"
    );
    Ok(())
}

/// Whether a media tool is installed. CI has no FFmpeg, so the tests that
/// need one skip rather than fail (same convention as the mix fixtures).
fn media_tool_available(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Write a recording fixture where `create_clip` resolves recordings (the
/// `SAKIOT_DATA_DIR` roots) and return its path. Callers remove it afterwards.
fn recording_fixture_path(stem: &str) -> Result<String, Box<dyn std::error::Error>> {
    let roots = web_server::audio::recording_path();
    let dir = web_server::audio::util::get_file_path_root(
        &roots,
        &(
            ALLOWED_GUILD_ID,
            ALLOWED_CHANNEL_ID,
            2026,
            5,
            stem.to_string(),
        ),
    );
    std::fs::create_dir_all(&dir)?;
    Ok(format!("{dir}/{stem}.ogg"))
}

/// Generate a real Ogg of `seconds` length at the handler's recording path.
fn generated_recording_fixture(
    stem: &str,
    seconds: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = recording_fixture_path(stem)?;
    let generated = std::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}"),
        ])
        .args(["-c:a", "libopus"])
        .arg(&path)
        .status()?;
    assert!(generated.success(), "ffmpeg must generate the fixture");
    Ok(path)
}

async fn seed_recording_row(pool: &PgPool, stem: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audio_files (file_name, guild_id, channel_id, user_id, year, month)
         VALUES ($1, $2, $3, $4, 2026, 5)",
    )
    .bind(stem)
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(USER_ID)
    .execute(pool)
    .await?;
    Ok(())
}

/// A range that starts inside a real recording but ends past its probed
/// duration must be refused after localization, and must not insert a row.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn create_clip_rejects_ranges_past_the_recording_duration(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !media_tool_available("ffmpeg") || !media_tool_available("ffprobe") {
        return Ok(());
    }
    seed_authorization_data(&pool).await?;
    let stem = "p0-1-duration-check";
    let source = generated_recording_fixture(stem, 2)?;
    seed_recording_row(&pool, stem).await?;

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
            .service(web::scope("/api").wrap(AuthMiddleware).service(create_clip)),
    )
    .await;
    let request = test::TestRequest::post()
        .uri(&format!(
            "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/2026/5/{stem}"
        ))
        .insert_header(("Cookie", access_cookie_value()?))
        .insert_header(("X-CSRF-Token", CSRF))
        .insert_header(("Idempotency-Key", "clip-duration-test"))
        .set_json(json!({"start": 0.0, "end": 10.0}))
        .to_request();
    let response = test::call_service(&app, request).await;
    let status = response.status();
    std::fs::remove_file(&source)?;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a range past the recording end must be rejected"
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM clips WHERE original_file_name = $1")
            .bind(stem)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, 0, "an over-long range must not insert a clip row");
    Ok(())
}

/// A recording ffprobe cannot read is an upstream media failure: the request
/// itself was valid, so the handler must answer 502 and store nothing.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn create_clip_reports_bad_gateway_when_the_recording_cannot_be_probed(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !media_tool_available("ffprobe") {
        return Ok(());
    }
    seed_authorization_data(&pool).await?;
    let stem = "p0-1-unprobeable";
    let source = recording_fixture_path(stem)?;
    std::fs::write(&source, b"not an ogg stream")?;
    seed_recording_row(&pool, stem).await?;

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
            .service(web::scope("/api").wrap(AuthMiddleware).service(create_clip)),
    )
    .await;
    let request = test::TestRequest::post()
        .uri(&format!(
            "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/2026/5/{stem}"
        ))
        .insert_header(("Cookie", access_cookie_value()?))
        .insert_header(("X-CSRF-Token", CSRF))
        .insert_header(("Idempotency-Key", "clip-probe-test"))
        .set_json(json!({"start": 0.0, "end": 1.0}))
        .to_request();
    let response = test::call_service(&app, request).await;
    let status = response.status();
    std::fs::remove_file(&source)?;

    assert_eq!(
        status,
        StatusCode::BAD_GATEWAY,
        "an unprobeable recording must be a bad gateway"
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM clips WHERE original_file_name = $1")
            .bind(stem)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, 0, "an unprobeable recording must not insert a row");
    Ok(())
}

/// The happy path still works after the new checks: a valid range inside the
/// recording produces a row whose length matches a non-empty file on disk.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn create_clip_stores_a_real_clip_for_a_valid_range(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !media_tool_available("ffmpeg") || !media_tool_available("ffprobe") {
        return Ok(());
    }
    seed_authorization_data(&pool).await?;
    let stem = "p0-1-valid-range";
    let source = generated_recording_fixture(stem, 2)?;
    seed_recording_row(&pool, stem).await?;

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
            .service(web::scope("/api").wrap(AuthMiddleware).service(create_clip)),
    )
    .await;
    let request = test::TestRequest::post()
        .uri(&format!(
            "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/2026/5/{stem}"
        ))
        .insert_header(("Cookie", access_cookie_value()?))
        .insert_header(("X-CSRF-Token", CSRF))
        .insert_header(("Idempotency-Key", "clip-valid-test"))
        .set_json(json!({"start": 0.5, "end": 1.5, "name": "valid clip"}))
        .to_request();
    let response = test::call_service(&app, request).await;
    let status = response.status();
    let body: serde_json::Value = test::read_body_json(response).await;
    std::fs::remove_file(&source)?;
    assert_eq!(
        status,
        StatusCode::OK,
        "valid range must create a clip: {body}"
    );

    let clip_id = body["id"]
        .as_str()
        .expect("clip id in response")
        .to_string();
    let stored_length: f32 = sqlx::query_scalar("SELECT length FROM clips WHERE clip_id = $1")
        .bind(&clip_id)
        .fetch_one(&pool)
        .await?;
    assert!(
        (stored_length - 1.0).abs() < 0.01,
        "stored length {stored_length} must match the requested 1s"
    );

    let saved = body["file"].as_str().expect("saved file in response");
    let clip_path = format!("{}{saved}", web_server::audio::clips_path());
    let clip_size = std::fs::metadata(&clip_path)?.len();
    assert!(clip_size > 0, "a stored clip must have bytes on disk");
    // The stored length must describe the audio that is actually there, not
    // just the requested range.
    let probed = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&clip_path)
        .output()?;
    let rendered: f64 = String::from_utf8_lossy(&probed.stdout)
        .trim()
        .parse()
        .expect("ffprobe must report the rendered clip duration");
    assert!(
        (rendered - 1.0).abs() < 0.1,
        "rendered clip is {rendered}s, expected the requested 1s"
    );
    std::fs::remove_file(&clip_path)?;
    Ok(())
}

/// `@everyone` may only apply to members. A stranger with no `user_guilds` row
/// must not inherit the guild's `@everyone` permissions — not even when those
/// include ADMINISTRATOR — while both owner paths keep full access.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn admin_endpoints_deny_everyone_permissions_to_non_members(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    // The most generous possible @everyone: membership alone decides access.
    sqlx::query("UPDATE roles SET permission = $1 WHERE guild_id = $2 AND role_id = $2")
        .bind(Permissions::all().bits())
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_guild_cooldown)
                    .service(set_guild_cooldown),
            ),
    )
    .await;
    let uri = format!("/api/admin/guilds/{ALLOWED_GUILD_ID}/cooldown");

    // No user_guilds row: denied on both read and write.
    let stranger = access_cookie_for(USER_ID + 1000)?;
    let request = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Cookie", stranger.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a non-member must not read admin cooldown settings"
    );

    let request = test::TestRequest::put()
        .uri(&uri)
        .insert_header(("Cookie", stranger))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"cooldown_seconds": 30}))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a non-member must not write admin cooldown settings"
    );
    let stored: Option<i32> =
        sqlx::query_scalar("SELECT cooldown_seconds FROM guild_jam_cooldowns WHERE guild_id = $1")
            .bind(ALLOWED_GUILD_ID)
            .fetch_optional(&pool)
            .await?;
    assert!(
        stored.is_none(),
        "a denied write must not change the cooldown"
    );

    // Member without personal roles: @everyone applies, so the write succeeds.
    let request = test::TestRequest::put()
        .uri(&uri)
        .insert_header(("Cookie", access_cookie_value()?))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"cooldown_seconds": 30}))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "a member holding @everyone's bits must still be allowed"
    );

    // Live owner (`guilds.owner_id`) with no membership row: allowed.
    let request = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Cookie", access_cookie_for(OTHER_USER_ID)?))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the live guild owner keeps access"
    );

    // Seeded owner flag on an existing membership row: allowed. Clear
    // @everyone first, or this member would pass on those bits alone and the
    // assertion would not exercise the owner flag at all.
    sqlx::query("UPDATE roles SET permission = 0 WHERE guild_id = $1 AND role_id = $1")
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE user_guilds SET owner = true WHERE id = $1 AND user_id = $2")
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .execute(&pool)
        .await?;
    let request = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Cookie", access_cookie_value()?))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the seeded owner grant keeps access"
    );
    Ok(())
}

/// Discord snowflakes exceed 2^53, so an override id must travel as a decimal
/// string: as a JSON number the browser rounds it and the delete request would
/// target a different user. It must round-trip through list and DELETE exactly.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn cooldown_overrides_keep_int64_user_ids_exact(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    const SNOWFLAKE: i64 = 9_007_199_254_740_993; // 2^53 + 1
    sqlx::query("UPDATE user_guilds SET owner = true WHERE id = $1 AND user_id = $2")
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .execute(&pool)
        .await?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(list_user_overrides)
                    .service(set_user_override)
                    .service(delete_user_override),
            ),
    )
    .await;
    let cookie = access_cookie_value()?;

    let request = test::TestRequest::put()
        .uri(&format!(
            "/api/admin/guilds/{ALLOWED_GUILD_ID}/cooldown/overrides/{SNOWFLAKE}"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"cooldown_seconds": 45}))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/admin/guilds/{ALLOWED_GUILD_ID}/cooldown/overrides"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let entry = body
        .as_array()
        .expect("override list is an array")
        .first()
        .expect("the override must be listed");
    assert_eq!(
        entry["user_id"],
        json!("9007199254740993"),
        "the id must be a decimal string, not a rounded JSON number"
    );
    assert_eq!(entry["cooldown_seconds"], json!(45));

    let request = test::TestRequest::delete()
        .uri(&format!(
            "/api/admin/guilds/{ALLOWED_GUILD_ID}/cooldown/overrides/{SNOWFLAKE}"
        ))
        .insert_header(("Cookie", cookie))
        .insert_header(("X-CSRF-Token", CSRF))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_jam_cooldown_overrides WHERE guild_id = $1")
            .bind(ALLOWED_GUILD_ID)
            .fetch_one(&pool)
            .await?;
    assert_eq!(remaining, 0, "the string id must delete the exact row");
    Ok(())
}

/// A finalized recording with no end timestamp has an unknown end. Falling
/// back to the start timestamp made it look zero seconds long.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn live_state_reports_no_end_for_a_finalized_recording_without_one(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let stem = "no-end-timestamp";
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts)
         VALUES ($1, $2, $3, $4, 2026, 5, 1000, NULL)",
    )
    .bind(stem)
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(USER_ID)
    .execute(&pool)
    .await?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(web::scope("/api").wrap(AuthMiddleware).service(live_state)),
    )
    .await;
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/2026/5/{stem}/state"
        ))
        .insert_header(("Cookie", access_cookie_value()?))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["started_at"], json!(1000));
    assert_eq!(
        body["ended_at"],
        serde_json::Value::Null,
        "an unknown end must stay unknown, not become the start timestamp"
    );
    Ok(())
}

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use actix_web::{App, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey};
use sakiot_proto::INTERNAL_SECRET_HEADER;
use sakiot_proto::fbi_agent::jam_response::JamResponseEnum;
use sakiot_proto::fbi_agent::jammer_server::{Jammer, JammerServer};
use sakiot_proto::fbi_agent::{JamData, JamResponse};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::RwLock;
use web_server::admin::cooldowns::{
    delete_user_override, get_guild_cooldown, list_user_overrides, set_guild_cooldown,
    set_user_override,
};
use web_server::admin::recording_policy::{
    RETENTION_RANGE_MESSAGE, get_recording_policy, put_recording_policy,
};
use web_server::admin::voice_settings::{
    delete_voice_settings, get_voice_settings, put_voice_settings,
};
use web_server::audio::{
    LiveContainer, SessionMixContainer, SilenceJobContainer, WaveformProgressContainer,
    create_session_clip, download_audio, download_session, generate_session_channel_mix, get_audio,
    get_clip_waveform_data, get_current_month_permission, get_live_stems, get_recording_events,
    get_session_channel_mix, get_session_channel_mix_media, get_session_events,
    get_session_manifest, get_session_segment, get_session_silence_free,
    get_session_silence_free_waveform, get_session_silence_removal_status, get_session_waveform,
    get_waveform_data, live_playlist, live_segment, live_state,
    rebuild_session_silence_free_waveform, rebuild_session_waveform, remove_session_silence,
    remove_silence, session_live_playlist, session_live_segment,
};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::clips::{
    create_clip, delete as delete_clip, get_clip, get_clips, play_clip, rename_clip,
};
use web_server::fbi_agent_registry::AgentGrpcRegistry;
use web_server::members::get_role_view;
use web_server::permissions::{Permissions, get_combined_perm_for_user, visible_channels_for_user};
use web_server::recording_deletion::{DeletionPolicy, delete_recording, get_recording_deletion};
use web_server::recording_opt_out::{get_recording_opt_out, put_recording_opt_out};
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
            "/api/audio/remove-silence/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/session-allowed-fragment"
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
async fn traversal_file_names_are_rejected_by_every_audio_handler(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Segment validation runs before authorization or any filesystem access,
    // so no seed data is needed: a traversal-shaped name must answer 400 no
    // matter what exists on disk or in the database. `%2F`/`%5C` matter
    // because actix percent-decodes captured path parameters, so the handlers
    // really receive `..`-with-separators here.
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
                    .service(get_audio)
                    .service(download_audio)
                    .service(get_waveform_data)
                    .service(get_clip_waveform_data)
                    .service(get_recording_events)
                    .service(remove_silence)
                    .service(live_playlist)
                    .service(live_state)
                    .service(live_segment)
                    .service(create_clip),
            ),
    )
    .await;

    for name in [
        "..%2F..%2Fsecret",
        "..",
        "..%5Csecret",
        "bad%27name",
        "bad%00name",
    ] {
        let get_uris = [
            format!("/api/audio/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"),
            format!("/api/download/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"),
            format!("/api/audio/waveform/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"),
            format!("/api/audio/events/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"),
            format!(
                "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}/playlist.m3u8"
            ),
            format!("/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}/state"),
            format!(
                "/api/audio/live/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}/seg_00000.m4s"
            ),
        ];
        for uri in get_uris {
            let request = test::TestRequest::get()
                .uri(&uri)
                .insert_header(("Cookie", cookie.clone()))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        }

        let post_uris = [
            format!(
                "/api/audio/remove-silence/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"
            ),
            format!(
                "/api/audio/clips/create/{ALLOWED_GUILD_ID}/{ALLOWED_CHANNEL_ID}/1970/1/{name}"
            ),
        ];
        for uri in post_uris {
            let request = test::TestRequest::post()
                .uri(&uri)
                .insert_header(("Cookie", cookie.clone()))
                .insert_header(("X-CSRF-Token", CSRF))
                .insert_header(("Idempotency-Key", "traversal-test"))
                .set_json(json!({"start": 0.0, "end": 1.0}))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        }
    }

    // Clip ids are interpolated into derived-artifact paths too.
    let request = test::TestRequest::get()
        .uri(&format!(
            "/api/audio/clips/waveform/{ALLOWED_GUILD_ID}/..%2F..%2Fsecret"
        ))
        .insert_header(("Cookie", cookie.clone()))
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

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

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recording_policy_and_deletion_require_live_manager_permission(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    sqlx::query("UPDATE roles SET permission = $1 WHERE role_id = $2")
        .bind(BASE_VOICE_PERMISSIONS | (1_i64 << 5))
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;
    let session_id: i64 = sqlx::query_scalar("INSERT INTO recording_sessions (guild_id,user_id,starting_channel_id,state,started_at,ended_at) VALUES ($1,$2,$3,'finalized',now()-interval '2 days',now()-interval '1 day') RETURNING id")
        .bind(ALLOWED_GUILD_ID).bind(USER_ID).bind(ALLOWED_CHANNEL_ID).fetch_one(&pool).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(DeletionPolicy::default()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_recording_policy)
                    .service(put_recording_policy)
                    .service(delete_recording)
                    .service(get_recording_deletion),
            ),
    )
    .await;
    let cookie = access_cookie_value()?;
    let policy_uri = format!("/api/admin/guilds/{ALLOWED_GUILD_ID}/recording-policy");
    let default: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&policy_uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(default["retention_days"], serde_json::Value::Null);
    assert_eq!(default["is_default"], true);
    assert!(
        default["channels"]
            .as_array()
            .is_some_and(|channels| !channels.is_empty())
    );
    let invalid = test::TestRequest::put()
        .uri(&policy_uri)
        .insert_header(("Cookie", cookie.clone()))
        .insert_header(("X-CSRF-Token", CSRF))
        .set_json(json!({"retention_days":0,"excluded_channel_ids":[]}))
        .to_request();
    let invalid = test::call_service(&app, invalid).await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let invalid: serde_json::Value = test::read_body_json(invalid).await;
    assert_eq!(invalid["code"], 400);
    assert_eq!(invalid["kind"], "invalid_request");
    assert_eq!(invalid["message"], RETENTION_RANGE_MESSAGE);

    // Middleware rejections follow the same body contract.
    let anonymous =
        test::call_service(&app, test::TestRequest::get().uri(&policy_uri).to_request()).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let anonymous: serde_json::Value = test::read_body_json(anonymous).await;
    assert_eq!(
        (anonymous["code"].clone(), anonymous["kind"].clone()),
        (json!(401), json!("unauthorized"))
    );
    let forged = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&policy_uri)
            .insert_header(("Cookie", cookie.clone()))
            .set_json(json!({"retention_days":30,"excluded_channel_ids":[]}))
            .to_request(),
    )
    .await;
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let forged: serde_json::Value = test::read_body_json(forged).await;
    assert_eq!(forged["kind"], "csrf_rejected");

    let updated: serde_json::Value = test::call_and_read_body_json(&app,
        test::TestRequest::put().uri(&policy_uri)
            .insert_header(("Cookie",cookie.clone())).insert_header(("X-CSRF-Token",CSRF))
            .set_json(json!({"retention_days":30,"excluded_channel_ids":[ALLOWED_CHANNEL_ID.to_string()]})).to_request()).await;
    assert_eq!(updated["retention_days"], 30);
    assert_eq!(
        updated["excluded_channel_ids"][0],
        ALLOWED_CHANNEL_ID.to_string()
    );

    let deletion_uri = format!("/api/admin/guilds/{ALLOWED_GUILD_ID}/recordings/{session_id}");
    let forbidden_permanent = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("{deletion_uri}?mode=permanent"))
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(forbidden_permanent.status(), StatusCode::FORBIDDEN);
    let forbidden_permanent: serde_json::Value = test::read_body_json(forbidden_permanent).await;
    assert_eq!(forbidden_permanent["kind"], "forbidden");
    let accepted = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&deletion_uri)
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let job: serde_json::Value = test::read_body_json(accepted).await;
    assert_eq!(job["recording_session_id"], session_id.to_string());
    let status_uri = job["status_url"].as_str().expect("status url");
    let status: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(status_uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(status["state"], "soft_deleted");
    assert_eq!(status["mode"], "soft");
    let hidden: bool = sqlx::query_scalar(
        "SELECT deletion_requested_at IS NOT NULL FROM recording_sessions WHERE id=$1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await?;
    assert!(hidden);
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM recording_sessions WHERE id=$1")
        .bind(session_id)
        .fetch_one(&pool)
        .await?;
    assert!(retained > 0);

    let enabled_app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(DeletionPolicy {
                allow_permanent: true,
            }))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(delete_recording),
            ),
    )
    .await;
    let explicit: serde_json::Value = test::call_and_read_body_json(
        &enabled_app,
        test::TestRequest::delete()
            .uri(&format!("{deletion_uri}?mode=permanent"))
            .insert_header(("Cookie", cookie.clone()))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(explicit["id"], job["id"]);
    assert_eq!(explicit["state"], "queued");
    assert_eq!(explicit["mode"], "permanent");

    sqlx::query("UPDATE roles SET permission = $1 WHERE role_id = $2")
        .bind(BASE_VOICE_PERMISSIONS)
        .bind(ALLOWED_GUILD_ID)
        .execute(&pool)
        .await?;
    for uri in [&policy_uri, status_uri] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(uri)
                .insert_header(("Cookie", cookie.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&deletion_uri)
            .insert_header(("Cookie", cookie))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

/// Stands in for the bot's Jammer service: answers with scripted responses in
/// order and records every request it receives, with the internal secret it
/// presented.
#[derive(Clone, Default)]
struct ScriptedBot {
    responses: Arc<Mutex<VecDeque<JamResponse>>>,
    requests: Arc<Mutex<Vec<JamData>>>,
    secrets: Arc<Mutex<Vec<Option<String>>>>,
}

#[tonic::async_trait]
impl Jammer for ScriptedBot {
    async fn jam_it(
        &self,
        request: tonic::Request<JamData>,
    ) -> Result<tonic::Response<JamResponse>, tonic::Status> {
        fn poisoned<T>(_: std::sync::PoisonError<T>) -> tonic::Status {
            tonic::Status::internal("scripted bot state poisoned")
        }
        let secret = request
            .metadata()
            .get(INTERNAL_SECRET_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.secrets.lock().map_err(poisoned)?.push(secret);
        self.requests
            .lock()
            .map_err(poisoned)?
            .push(request.into_inner());
        let response = self
            .responses
            .lock()
            .map_err(poisoned)?
            .pop_front()
            .ok_or_else(|| tonic::Status::internal("no scripted response left"))?;
        Ok(tonic::Response::new(response))
    }
}

/// Serves `bot` on an ephemeral loopback port and returns its gRPC address.
async fn serve_bot(bot: ScriptedBot) -> Result<String, Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = format!("http://{}", listener.local_addr()?);
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(JammerServer::new(bot))
            .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener)),
    );
    Ok(address)
}

fn play_request(uri: &str) -> Result<test::TestRequest, Box<dyn std::error::Error>> {
    Ok(test::TestRequest::post()
        .uri(uri)
        .insert_header(("Cookie", access_cookie_value()?))
        .insert_header(("X-CSRF-Token", CSRF)))
}

/// The status, `Retry-After` header, and JSON body of a playback response.
async fn play_outcome(
    resp: actix_web::dev::ServiceResponse,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get(actix_web::http::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    (status, retry_after, test::read_body_json(resp).await)
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn clip_playback_reports_every_bot_outcome_through_the_contract(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let bot = ScriptedBot::default();
    let outcomes = [
        (JamResponseEnum::Ok, 0),
        (JamResponseEnum::NotPresent, 0),
        (JamResponseEnum::Cooldown, 12),
        (JamResponseEnum::Unknown, 0),
    ];
    bot.responses
        .lock()
        .unwrap()
        .extend(outcomes.map(|(resp, seconds)| JamResponse {
            resp: resp.into(),
            cooldown_remaining_seconds: seconds,
        }));
    let address = serve_bot(bot.clone()).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(AgentGrpcRegistry::new(
                &address,
                Some("registry-secret".into()),
            )))
            .service(web::scope("/api").wrap(AuthMiddleware).service(play_clip)),
    )
    .await;
    let uri = format!("/api/audio/clips/{ALLOWED_GUILD_ID}/own-clip/play");

    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "queued" }));

    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["kind"], "bot_not_in_voice");

    let (status, retry_after, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["kind"], "jam_cooldown");
    assert_eq!(body["retry_after_seconds"], 12);
    assert_eq!(retry_after.as_deref(), Some("12"));

    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["kind"], "clip_playback_failed");

    let requests = bot.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), outcomes.len());
    for request in requests {
        assert_eq!(
            (
                request.clip_name.as_str(),
                request.guild_id,
                request.user_id
            ),
            ("own-clip", ALLOWED_GUILD_ID, USER_ID),
            "the bot receives the resolved clip id and the caller"
        );
    }
    assert_eq!(
        *bot.secrets.lock().unwrap(),
        vec![Some("registry-secret".to_owned()); outcomes.len()],
        "every call presents the configured internal secret"
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn clip_playback_rejects_unplayable_requests_before_reaching_the_bot(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let bot = ScriptedBot::default();
    let address = serve_bot(bot.clone()).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(AgentGrpcRegistry::new(&address, None)))
            .service(web::scope("/api").wrap(AuthMiddleware).service(play_clip)),
    )
    .await;

    // A clip name is not a clip id: playback never falls back to a name lookup.
    let uri = format!("/api/audio/clips/{ALLOWED_GUILD_ID}/no-such-clip/play");
    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["kind"], "clip_not_found");

    let uri = format!("/api/audio/clips/{FORBIDDEN_GUILD_ID}/forbidden-clip/play");
    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["kind"], "forbidden");

    assert!(
        bot.requests.lock().unwrap().is_empty(),
        "rejected requests must not reach the bot"
    );
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn clip_playback_reports_an_unreachable_bot(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    // Nothing listens on a port that was bound and immediately released.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .app_data(web::Data::new(AgentGrpcRegistry::new(
                &format!("http://{closed}"),
                None,
            )))
            .service(web::scope("/api").wrap(AuthMiddleware).service(play_clip)),
    )
    .await;

    let uri = format!("/api/audio/clips/{ALLOWED_GUILD_ID}/own-clip/play");
    let (status, _, body) =
        play_outcome(test::call_service(&app, play_request(&uri)?.to_request()).await).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["kind"], "bot_unavailable");
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
        .uri("/api/audio/remove-silence/2/200/2026/5/forbidden-rec")
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

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn members_manage_only_their_own_recording_opt_out(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_recording_opt_out)
                    .service(put_recording_opt_out),
            ),
    )
    .await;
    let cookie = access_cookie_value()?;
    let uri = format!("/api/users/current/guilds/{ALLOWED_GUILD_ID}/recording-opt-out");

    // Recording is on by default.
    let initial: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(initial, json!({"opted_out": false}));

    // A change is a write, so it needs the CSRF token like every other one.
    let forged = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .set_json(json!({"opted_out": true}))
            .to_request(),
    )
    .await;
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);

    // Setting the same value twice is harmless.
    for opted_out in [true, true, false] {
        let stored: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::put()
                .uri(&uri)
                .insert_header(("Cookie", cookie.clone()))
                .insert_header(("X-CSRF-Token", CSRF))
                .set_json(json!({ "opted_out": opted_out }))
                .to_request(),
        )
        .await;
        assert_eq!(stored, json!({ "opted_out": opted_out }));
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM recording_opt_outs WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(ALLOWED_GUILD_ID)
        .bind(USER_ID)
        .fetch_one(&pool)
        .await?;
        assert_eq!(rows, i64::from(opted_out));
    }

    // A server the user is not a member of is off limits either way.
    let foreign_uri = format!("/api/users/current/guilds/{FORBIDDEN_GUILD_ID}/recording-opt-out");
    let foreign_get = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&foreign_uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(foreign_get.status(), StatusCode::FORBIDDEN);
    let foreign_put = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&foreign_uri)
            .insert_header(("Cookie", cookie))
            .insert_header(("X-CSRF-Token", CSRF))
            .set_json(json!({"opted_out": true}))
            .to_request(),
    )
    .await;
    assert_eq!(foreign_put.status(), StatusCode::FORBIDDEN);
    let foreign_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recording_opt_outs WHERE guild_id = $1")
            .bind(FORBIDDEN_GUILD_ID)
            .fetch_one(&pool)
            .await?;
    assert_eq!(foreign_rows, 0);
    Ok(())
}

// ---- role preview is clipped to the manager's own channels ----

const PRIVATE_CHANNEL_ID: i64 = ALLOWED_CHANNEL_ID + 50;
const INSIDER_ROLE_ID: i64 = 1101;
const MANAGER_ROLE_ID: i64 = 1102;
const ADMIN_ROLE_ID: i64 = 1103;
const MANAGER_ID: i64 = 30;
const ADMIN_ID: i64 = 40;
const MANAGE_GUILD_PERMISSION: i64 = 1 << 5;
const ADMINISTRATOR_PERMISSION: i64 = 1 << 3;

/// Guild 1 gains a private channel P that `@everyone` cannot view and the
/// Insiders role can. A MANAGE_GUILD-only manager cannot view P; an
/// Administrator can. P and the public channel each hold a finalized session,
/// a live stem, a clip and a stamp, and one more session moves from the
/// public channel into P.
async fn seed_role_preview_data(pool: &PgPool) -> Result<(), sqlx::Error> {
    seed_authorization_data(pool).await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 2, 'private')",
    )
    .bind(PRIVATE_CHANNEL_ID)
    .bind(ALLOWED_GUILD_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $2, 0, 'Insiders'), ($1, $3, $4, 'Managers'), ($1, $5, $6, 'Admins')",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(INSIDER_ROLE_ID)
    .bind(MANAGER_ROLE_ID)
    .bind(MANAGE_GUILD_PERMISSION)
    .bind(ADMIN_ROLE_ID)
    .bind(ADMINISTRATOR_PERMISSION)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $3), ($1, $4, 'role', $3, 0)",
    )
    .bind(PRIVATE_CHANNEL_ID)
    .bind(ALLOWED_GUILD_ID)
    .bind(VIEW_CHANNEL_PERMISSION)
    .bind(INSIDER_ROLE_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         VALUES ($1, $2, 'allowed guild', NULL, false, 0, ARRAY[]::text[]),
                ($1, $3, 'allowed guild', NULL, false, 0, ARRAY[]::text[])",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(MANAGER_ID)
    .bind(ADMIN_ID)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2), ($3, $4)")
        .bind(MANAGER_ID)
        .bind(MANAGER_ROLE_ID)
        .bind(ADMIN_ID)
        .bind(ADMIN_ROLE_ID)
        .execute(pool)
        .await?;

    // Finalized sessions: one per channel, plus one that moves from the
    // public channel into P.
    for (index, start, fragment, started_at_s) in [
        (0, ALLOWED_CHANNEL_ID, ALLOWED_CHANNEL_ID, 1.0),
        (1, PRIVATE_CHANNEL_ID, PRIVATE_CHANNEL_ID, 11.0),
        (2, ALLOWED_CHANNEL_ID, PRIVATE_CHANNEL_ID, 21.0),
    ] {
        let started_at_ms = (index * 10 + 1) * 1000;
        let session_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO recording_sessions
                (guild_id, user_id, starting_channel_id, current_channel_id, state,
                 started_at, ended_at, end_reason, last_segment_index)
             VALUES ($1, $2, $3, $4, 'finalized',
                     to_timestamp($5), to_timestamp($5 + 2), 'test', 1)
             RETURNING id",
        )
        .bind(ALLOWED_GUILD_ID)
        .bind(OTHER_USER_ID)
        .bind(start)
        .bind(fragment)
        .bind(started_at_s)
        .fetch_one(pool)
        .await?;
        for (segment, channel) in [(0, start), (1, fragment)] {
            sqlx::query(
                "INSERT INTO audio_files
                    (file_name, guild_id, channel_id, user_id, year, month,
                     start_ts, end_ts, recording_session_id, segment_index)
                 VALUES ($1, $2, $3, $4, 1970, 1, $5, $5 + 1000, $6, $7)",
            )
            .bind(format!("session-{index}-{segment}"))
            .bind(ALLOWED_GUILD_ID)
            .bind(channel)
            .bind(OTHER_USER_ID)
            .bind(started_at_ms + i64::from(segment) * 1000)
            .bind(session_id)
            .bind(segment)
            .execute(pool)
            .await?;
        }
    }

    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state)
         VALUES ('preview-bot', 'active', 'active')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts,
             recording_owner_instance_id, recording_heartbeat_at)
         VALUES ('live-public', $1, $2, $4, 2026, 5, 1000, 'preview-bot', now()),
                ('live-private', $1, $3, $4, 2026, 5, 1000, 'preview-bot', now())",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(PRIVATE_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO clips
            (clip_id, guild_id, channel_id, user_id, saved_file_name, start_time)
         VALUES ('public-clip', $1, $2, $4, '2026/05/public.ogg', 0),
                ('private-clip', $1, $3, $4, '2026/05/private.ogg', 0)",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(PRIVATE_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO stamps (guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts, note)
         VALUES ($1, $2, $4, $4, 1000, 'public-stamp'),
                ($1, $3, $4, $4, 1000, 'private-stamp')",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(ALLOWED_CHANNEL_ID)
    .bind(PRIVATE_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .execute(pool)
    .await?;
    Ok(())
}

/// What one viewer's role preview of Insiders exposes on every `as_role` path.
#[derive(Debug, PartialEq)]
struct PreviewExposure {
    /// (file, channel journey, access annotation) per listed recording.
    sessions: Vec<(String, Vec<String>, String)>,
    live_stems: Vec<String>,
    clips: Vec<String>,
    stamps: Vec<String>,
    role_view_channels: Vec<String>,
}

async fn preview_exposure(
    pool: &PgPool,
    viewer: i64,
) -> Result<PreviewExposure, Box<dyn std::error::Error>> {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_live_stems)
                    .service(get_current_month_permission)
                    .service(get_clips)
                    .service(get_stamps)
                    .service(get_role_view),
            ),
    )
    .await;
    let cookie = access_cookie_for(viewer)?;
    let get = async |uri: String| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let request = test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        Ok(test::read_body_json(response).await)
    };
    let as_role = format!("as_role={INSIDER_ROLE_ID}");

    let tree = get(format!("/api/current/{ALLOWED_GUILD_ID}?{as_role}")).await?;
    let mut sessions = Vec::new();
    for channel in tree.as_array().into_iter().flatten() {
        for dir in channel["dirs"].as_array().into_iter().flatten() {
            for files in dir["months"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                for file in files.as_array().into_iter().flatten() {
                    let journey = file["channel_journey"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect();
                    sessions.push((
                        file["file"].as_str().unwrap_or_default().to_string(),
                        journey,
                        file["access"].as_str().unwrap_or_default().to_string(),
                    ));
                }
            }
        }
    }
    sessions.sort();

    let strings = |value: serde_json::Value, field: Option<&str>| -> Vec<String> {
        let mut out: Vec<String> = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| match field {
                Some(field) => item[field].as_str().map(str::to_string),
                None => item.as_str().map(str::to_string),
            })
            .collect();
        out.sort();
        out
    };
    let live_stems = strings(
        get(format!(
            "/api/current/{ALLOWED_GUILD_ID}/live-stems?{as_role}"
        ))
        .await?,
        None,
    );
    let clips = strings(
        get(format!("/api/audio/clips/{ALLOWED_GUILD_ID}?{as_role}")).await?,
        Some("clip_id"),
    );
    let stamps = strings(
        get(format!("/api/stamps/{ALLOWED_GUILD_ID}?{as_role}")).await?,
        Some("note"),
    );
    let role_view = get(format!(
        "/api/admin/guilds/{ALLOWED_GUILD_ID}/roles/{INSIDER_ROLE_ID}/channels"
    ))
    .await?;
    let role_view_channels = strings(role_view["channels"].clone(), Some("channel_id"));

    Ok(PreviewExposure {
        sessions,
        live_stems,
        clips,
        stamps,
        role_view_channels,
    })
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn role_preview_never_shows_channels_the_manager_cannot_view(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_role_preview_data(&pool).await?;
    let public = ALLOWED_CHANNEL_ID.to_string();
    let private = PRIVATE_CHANNEL_ID.to_string();
    let strings = |items: &[&str]| -> Vec<String> { items.iter().map(|s| s.to_string()).collect() };
    let listed = |file: &str, journey: &[&String]| {
        (
            file.to_string(),
            journey.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            "can-listen".to_string(),
        )
    };

    // The MANAGE_GUILD-only manager cannot view P, so nothing from P reaches
    // them, including the session that moved from the public channel into P.
    // (`own-clip` is the seed's public clip.)
    assert_eq!(
        preview_exposure(&pool, MANAGER_ID).await?,
        PreviewExposure {
            sessions: vec![
                listed("live-public.ogg", &[&public]),
                listed("session-0-0.ogg", &[&public]),
            ],
            live_stems: strings(&["live-public"]),
            clips: strings(&["own-clip", "public-clip"]),
            stamps: strings(&["public-stamp"]),
            role_view_channels: vec![public.clone()],
        }
    );

    // An Administrator views every channel, so their preview is unclipped.
    assert_eq!(
        preview_exposure(&pool, ADMIN_ID).await?,
        PreviewExposure {
            sessions: vec![
                listed("live-private.ogg", &[&private]),
                listed("live-public.ogg", &[&public]),
                listed("session-0-0.ogg", &[&public]),
                listed("session-1-0.ogg", &[&private]),
                listed("session-2-0.ogg", &[&public, &private]),
            ],
            live_stems: strings(&["live-private", "live-public"]),
            clips: strings(&["own-clip", "private-clip", "public-clip"]),
            stamps: strings(&["private-stamp", "public-stamp"]),
            role_view_channels: vec![public, private],
        }
    );
    Ok(())
}

// ---- stage channels follow the same visibility as voice channels ----

const STAGE_CHANNEL_ID: i64 = 300;
const STAGE_MUTED_ROLE_ID: i64 = 1201;
const ROLE_DENIED_ID: i64 = 31;
const MEMBER_DENIED_ID: i64 = 32;

/// Guild 1 gains a stage channel (type 13) holding a finalized session with
/// one fragment and a live stem. `@everyone` can view and join it; one member
/// loses CONNECT through a role overwrite and another loses VIEW_CHANNEL
/// through a member overwrite. The guild owner gets a membership row.
async fn seed_stage_channel_data(pool: &PgPool) -> Result<i64, sqlx::Error> {
    seed_authorization_data(pool).await?;
    // No `channel_type` row: that foreign key was dropped so unknown channel
    // types can be cached.
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $2, 13, 'stage')",
    )
    .bind(STAGE_CHANNEL_ID)
    .bind(ALLOWED_GUILD_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $2, 0, 'Stage muted')",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(STAGE_MUTED_ROLE_ID)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $3), ($1, $4, 'user', 0, $5)",
    )
    .bind(STAGE_CHANNEL_ID)
    .bind(STAGE_MUTED_ROLE_ID)
    .bind(CONNECT_PERMISSION)
    .bind(MEMBER_DENIED_ID)
    .bind(VIEW_CHANNEL_PERMISSION)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, icon, owner, permissions, features)
         SELECT $1, user_id, 'allowed guild', NULL, false, 0, ARRAY[]::text[]
           FROM unnest($2::bigint[]) AS user_id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(vec![OTHER_USER_ID, ROLE_DENIED_ID, MEMBER_DENIED_ID])
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(ROLE_DENIED_ID)
        .bind(STAGE_MUTED_ROLE_ID)
        .execute(pool)
        .await?;

    let session_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, end_reason, last_segment_index)
         VALUES ($1, $2, $3, $3, 'finalized',
                 to_timestamp(1), to_timestamp(3), 'test', 0)
         RETURNING id",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(OTHER_USER_ID)
    .bind(STAGE_CHANNEL_ID)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month,
             start_ts, end_ts, recording_session_id, segment_index)
         VALUES ('stage-fragment', $1, $2, $3, 1970, 1, 1000, 3000, $4, 0)",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(STAGE_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .bind(session_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state)
         VALUES ('stage-bot', 'active', 'active')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts,
             recording_owner_instance_id, recording_heartbeat_at)
         VALUES ('stage-live', $1, $2, $3, 2026, 5, 5000, 'stage-bot', now())",
    )
    .bind(ALLOWED_GUILD_ID)
    .bind(STAGE_CHANNEL_ID)
    .bind(OTHER_USER_ID)
    .execute(pool)
    .await?;
    Ok(session_id)
}

/// What one viewer can reach of the stage channel's recordings.
#[derive(Debug, PartialEq)]
struct StageExposure {
    /// (file, access annotation) per recording in the stage channel's tree.
    listed: Vec<(String, Option<String>)>,
    live_stems: Vec<String>,
    manifest: StatusCode,
    events: StatusCode,
}

async fn stage_exposure(
    pool: &PgPool,
    session_id: i64,
    viewer: i64,
    as_role: Option<i64>,
) -> Result<StageExposure, Box<dyn std::error::Error>> {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(get_current_month_permission)
                    .service(get_live_stems)
                    .service(get_session_manifest)
                    .service(get_session_events),
            ),
    )
    .await;
    let cookie = access_cookie_for(viewer)?;
    let call = async |uri: String| {
        let request = test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Cookie", cookie.clone()))
            .to_request();
        test::call_service(&app, request).await
    };
    let query = as_role.map_or_else(String::new, |role| format!("?as_role={role}"));

    let response = call(format!("/api/current/{ALLOWED_GUILD_ID}{query}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree: serde_json::Value = test::read_body_json(response).await;
    let mut listed = Vec::new();
    for channel in tree.as_array().into_iter().flatten() {
        if channel["channel_id"] != json!(STAGE_CHANNEL_ID.to_string()) {
            continue;
        }
        for dir in channel["dirs"].as_array().into_iter().flatten() {
            for files in dir["months"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                for file in files.as_array().into_iter().flatten() {
                    listed.push((
                        file["file"].as_str().unwrap_or_default().to_string(),
                        file["access"].as_str().map(str::to_string),
                    ));
                }
            }
        }
    }
    listed.sort();

    let response = call(format!("/api/current/{ALLOWED_GUILD_ID}/live-stems{query}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let live_stems: Vec<String> = test::read_body_json(response).await;

    Ok(StageExposure {
        listed,
        live_stems,
        manifest: call(format!("/api/audio/sessions/{session_id}/manifest"))
            .await
            .status(),
        events: call(format!("/api/audio/sessions/{session_id}/events"))
            .await
            .status(),
    })
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn stage_channel_recordings_follow_view_and_connect(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let session_id = seed_stage_channel_data(&pool).await?;
    let entry = |file: &str, access: Option<&str>| (file.to_string(), access.map(str::to_string));
    let visible = StageExposure {
        listed: vec![
            entry("stage-fragment.ogg", None),
            entry("stage-live.ogg", None),
        ],
        live_stems: vec!["stage-live".to_string()],
        manifest: StatusCode::OK,
        events: StatusCode::OK,
    };
    let can_listen = StageExposure {
        listed: vec![
            entry("stage-fragment.ogg", Some("can-listen")),
            entry("stage-live.ogg", Some("can-listen")),
        ],
        live_stems: vec!["stage-live".to_string()],
        manifest: StatusCode::OK,
        events: StatusCode::OK,
    };
    let hidden = StageExposure {
        listed: Vec::new(),
        live_stems: Vec::new(),
        manifest: StatusCode::FORBIDDEN,
        events: StatusCode::FORBIDDEN,
    };

    // A member with VIEW_CHANNEL and CONNECT, and the owner, see the stage
    // channel's recordings like any voice channel's.
    assert_eq!(
        stage_exposure(&pool, session_id, USER_ID, None).await?,
        visible
    );
    assert_eq!(
        stage_exposure(&pool, session_id, OTHER_USER_ID, None).await?,
        visible
    );
    // Role and member denies on the stage channel are honored.
    assert_eq!(
        stage_exposure(&pool, session_id, ROLE_DENIED_ID, None).await?,
        hidden
    );
    assert_eq!(
        stage_exposure(&pool, session_id, MEMBER_DENIED_ID, None).await?,
        hidden
    );

    // Role preview annotates stage recordings: @everyone can listen, and the
    // role denied CONNECT can only see them. Media checks stay on the
    // owner's own access.
    assert_eq!(
        stage_exposure(&pool, session_id, OTHER_USER_ID, Some(ALLOWED_GUILD_ID)).await?,
        can_listen
    );
    assert_eq!(
        stage_exposure(&pool, session_id, OTHER_USER_ID, Some(STAGE_MUTED_ROLE_ID)).await?,
        StageExposure {
            listed: vec![
                entry("stage-fragment.ogg", Some("visible-only")),
                entry("stage-live.ogg", Some("visible-only")),
            ],
            live_stems: Vec::new(),
            manifest: StatusCode::OK,
            events: StatusCode::OK,
        }
    );
    Ok(())
}

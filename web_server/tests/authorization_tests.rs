use std::collections::HashMap;

use actix_web::{http::StatusCode, test, web, App};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::RwLock;
use web_server::audio::{
    download_audio, get_audio, get_waveform_data, live_playlist, live_segment, live_state,
    remove_silence, LiveContainer, SilenceJobContainer, WaveformProgressContainer,
};
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::clips::{create_clip, delete as delete_clip, get_clip, get_clips};
use web_server::stamps::get_stamps;

const USER_ID: i64 = 10;
const OTHER_USER_ID: i64 = 20;
const ALLOWED_GUILD_ID: i64 = 1;
const FORBIDDEN_GUILD_ID: i64 = 2;
const ALLOWED_CHANNEL_ID: i64 = 100;
const FORBIDDEN_CHANNEL_ID: i64 = 200;
const CONNECT_PERMISSION: i64 = 1 << 20;
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
        CONNECT_PERMISSION,
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
async fn forbidden_cross_guild_requests_are_rejected(
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    seed_authorization_data(&pool).await?;
    let cookie = access_cookie_value()?;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(access_keys()))
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

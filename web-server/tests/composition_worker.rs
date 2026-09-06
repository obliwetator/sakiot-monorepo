//! Exercise the real child executable and media tools against disposable SQLx
//! databases and a temporary data root. No runtime database or media is used.
use serde_json::json;
use sqlx::{ConnectOptions, PgPool, Row};
use std::{os::unix::process::CommandExt, process::Stdio, time::Duration};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn seed(
    pool: &PgPool,
    root: &std::path::Path,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let clips = root.join("clips");
    std::fs::create_dir_all(&clips)?;
    let result = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-ac",
            "2",
            "-ar",
            "48000",
        ])
        .arg(clips.join("source.ogg"))
        .status()?;
    assert!(result.success());
    sqlx::raw_sql("INSERT INTO guilds (id, owner_id) VALUES (1,100);
        INSERT INTO user_guilds (id,user_id,name,owner,permissions,features) VALUES (1,100,'Test',true,8,'{}');
        INSERT INTO channels (channel_id,guild_id,type,name) VALUES (10,1,2,'Test');
        INSERT INTO clips (clip_id,guild_id,user_id,channel_id,start_time,length,saved_file_name,original_file_name)
        VALUES ('source',1,100,10,0,1,'source.ogg','recording');").execute(pool).await?;
    let body = json!({"name":"Durable export", "master_volume_db":0, "segments":[{
        "source":"clip", "source_id":"source", "source_in":0, "source_out":0.5,
        "timeline_start":0, "track":0,
        "effects":{"volume_db":0,"pitch_cents":0,"rate":1,"bass_db":0,"mid_db":0,"treble_db":0}
    }]});
    let snapshot = json!({"body":body,"sources":["source.ogg"],"overwrite":null,"channel_id":10,"name":"Durable export"});
    let id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO composition_jobs (id,guild_id,user_id,idempotency_key,request,snapshot,result_clip_id,state,attempts,attempt_token,lease_expires_at) VALUES ($1,1,100,$1,$2,$3,'rendered','running',1,$4,now()+interval '60 seconds')")
        .bind(&id).bind(body).bind(snapshot).bind(&token).execute(pool).await?;
    Ok((id, token))
}

fn command(pool: &PgPool, root: &std::path::Path, id: &str, token: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_web_server"));
    command
        .args(["compose-worker", id, token])
        .env(
            "DATABASE_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env("SAKIOT_DATA_DIR", root)
        .env("SAKIOT_MEDIA_ARCHIVE_ENABLED", "false")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command.process_group(0);
    command
}

fn run(
    mut command: std::process::Command,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let mut child = command.spawn()?;
    let start = std::time::Instant::now();
    while child.try_wait()?.is_none() {
        if start.elapsed() > Duration::from_secs(30) {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            return Err("Composition child exceeded test deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(child.wait_with_output()?)
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn real_worker_renders_and_publishes_once(pool: PgPool) -> TestResult {
    let root = tempfile::tempdir()?;
    let (id, token) = seed(&pool, root.path()).await?;
    let output = run(command(&pool, root.path(), &id, &token))?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let row = sqlx::query("SELECT j.state,c.saved_file_name,c.length,m.state AS archive_state,m.clip_saved_file_name FROM composition_jobs j JOIN clips c ON c.clip_id=j.result_clip_id JOIN media_objects m ON m.clip_id=c.clip_id WHERE j.id=$1")
        .bind(&id).fetch_one(&pool).await?;
    assert_eq!(row.get::<String, _>("state"), "ready");
    assert_eq!(row.get::<String, _>("archive_state"), "pending");
    let saved: String = row.try_get("saved_file_name")?;
    assert_eq!(row.get::<String, _>("clip_saved_file_name"), saved);
    assert!(root.path().join("clips").join(&saved).is_file());
    assert!((row.get::<f32, _>("length") - 0.5).abs() < 0.02);
    let duplicate = run(command(&pool, root.path(), &id, &token))?;
    assert!(!duplicate.status.success());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM clips WHERE clip_id='rendered'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn killed_worker_can_restart_with_a_new_attempt(pool: PgPool) -> TestResult {
    let root = tempfile::tempdir()?;
    let (id, token) = seed(&pool, root.path()).await?;
    // Hold the source row's table to stop the child inside preparation. This
    // makes the crash deterministic without relying on render duration.
    let mut lock = pool.begin().await?;
    sqlx::query("LOCK TABLE clips IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    let mut child = command(&pool, root.path(), &id, &token).spawn()?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    child.wait()?;
    lock.rollback().await?;
    let retry = uuid::Uuid::new_v4().to_string();
    sqlx::query("UPDATE composition_jobs SET attempts=2,attempt_token=$2,lease_expires_at=now()+interval '60 seconds' WHERE id=$1")
        .bind(&id).bind(&retry).execute(&pool).await?;
    let output = run(command(&pool, root.path(), &id, &retry))?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state: String = sqlx::query_scalar("SELECT state FROM composition_jobs WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(state, "ready");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn http_admission_is_durable_idempotent_and_does_not_load_media(pool: PgPool) -> TestResult {
    use actix_web::{App, HttpMessage, dev::Service, test as http, web};
    use web_server::{
        auth::{Access, AccessKeys, AuthKind, Token},
        clip_editor::{compose_clip, compose_clip_status},
    };
    let keys = AccessKeys {
        access_encode: jsonwebtoken::EncodingKey::from_secret(b"test"),
        refresh_encode: jsonwebtoken::EncodingKey::from_secret(b"test"),
        access_decode: jsonwebtoken::DecodingKey::from_secret(b"test"),
        refresh_decode: jsonwebtoken::DecodingKey::from_secret(b"test"),
    };
    let token = Token::<Access>::decode(
        &Token::<Access>::encode(100, AuthKind::Discord, "csrf".into(), &keys.access_encode)?,
        &keys,
    )?;
    sqlx::raw_sql("INSERT INTO guilds (id,owner_id) VALUES (1,100);
        INSERT INTO user_guilds (id,user_id,name,owner,permissions,features) VALUES (1,100,'Test',true,8,'{}');
        INSERT INTO channels (channel_id,guild_id,type) VALUES (10,1,2);
        INSERT INTO clips (clip_id,guild_id,user_id,channel_id,start_time,saved_file_name,original_file_name)
        VALUES ('source',1,100,10,0,'deliberately-not-on-disk.ogg','recording');").execute(&pool).await?;
    let app = http::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .wrap_fn(move |req, service| {
                req.extensions_mut().insert(token.clone());
                service.call(req)
            })
            .service(compose_clip)
            .service(compose_clip_status),
    )
    .await;
    let body = json!({"master_volume_db":0,"segments":[{
        "source":"clip","source_id":"source","source_in":0,"source_out":1,"timeline_start":0,"track":0,
        "effects":{"volume_db":0,"pitch_cents":0,"rate":1,"bass_db":0,"mid_db":0,"treble_db":0}
    }]});
    let mut id = None;
    for _ in 0..2 {
        let req = http::TestRequest::post()
            .uri("/audio/clips/1/compose")
            .insert_header(("Idempotency-Key", "request-key"))
            .set_json(&body)
            .to_request();
        let response = http::call_service(&app, req).await;
        assert_eq!(response.status(), 202);
        let value: serde_json::Value = http::read_body_json(response).await;
        if let Some(id) = &id {
            assert_eq!(id, &value["id"]);
        }
        id = Some(value["id"].clone());
    }
    let id = id.unwrap();
    let id = id.as_str().unwrap();
    let response = http::call_service(
        &app,
        http::TestRequest::get()
            .uri(&format!("/audio/clips/1/compose/{id}"))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let value: serde_json::Value = http::read_body_json(response).await;
    assert_eq!(value["status"], "queued");
    let response = http::call_service(
        &app,
        http::TestRequest::get()
            .uri(&format!("/audio/clips/2/compose/{id}"))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 404);
    Ok(())
}

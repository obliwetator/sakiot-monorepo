//! `/api/realtime` end to end: real sockets, the real triggers and listener.
//! Each viewer receives refresh signals only for what they may list, a
//! permission change re-authorizes them, and shutdown closes sockets at once.

mod support;

use std::time::Duration;

use actix_web::{App, web};
use awc::ws::{Codec, Frame, Message};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{Value, json};
use sqlx::PgPool;
use web_server::auth::cookies::ACCESS_TOKEN_COOKIE;
use web_server::auth::{Access, AccessKeys, AuthKind, AuthMiddleware, Token};
use web_server::config::Config;
use web_server::realtime::{Hub, realtime_socket};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Socket = actix_codec::Framed<awc::BoxedSocket, Codec>;

const GUILD: i64 = 1;
const PUBLIC: i64 = 100;
const PRIVATE: i64 = 150;
const INSIDER_ROLE: i64 = 1101;
const OWNER: i64 = 20;
const VIEWER: i64 = 10;
const INSIDER: i64 = 11;
const ORIGIN: &str = "https://app.example.com";
const VIEW_CHANNEL: i64 = 1 << 10;
const CONNECT: i64 = 1 << 20;
const WAIT: Duration = Duration::from_secs(5);

fn keys() -> AccessKeys {
    AccessKeys {
        access_encode: EncodingKey::from_secret(b"test_secret"),
        refresh_encode: EncodingKey::from_secret(b"test_secret"),
        access_decode: DecodingKey::from_secret(b"test_secret"),
        refresh_decode: DecodingKey::from_secret(b"test_secret"),
    }
}

fn config(realtime_enabled: bool) -> Config {
    Config {
        database_url: String::new(),
        client_id: String::new(),
        client_secret: String::new(),
        access_secret: String::new(),
        refresh_secret: String::new(),
        dev_account_id: 0,
        dev_login_secret: None,
        cors_allowed_origin: ORIGIN.into(),
        oauth_allowed_opener_origins: Vec::new(),
        oauth_allowed_opener_host_suffixes: Vec::new(),
        cookie_domain: String::new(),
        discord_redirect_uri: String::new(),
        grpc_address: String::new(),
        fbi_agent_registry_secret: None,
        host: String::new(),
        port: 0,
        db_max_connections: 5,
        recording_permanent_delete_enabled: false,
        server_timing_header: false,
        realtime_enabled,
    }
}

async fn seed(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, $2)")
        .bind(GUILD)
        .bind(OWNER)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name)
         VALUES ($1, $1, $3, '@everyone'), ($1, $2, 0, 'Insiders')",
    )
    .bind(GUILD)
    .bind(INSIDER_ROLE)
    .bind(VIEW_CHANNEL | CONNECT)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         VALUES ($1, $3, 2, 'public'), ($2, $3, 2, 'private')",
    )
    .bind(PUBLIC)
    .bind(PRIVATE)
    .bind(GUILD)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         VALUES ($1, $2, 'role', 0, $4), ($1, $3, 'role', $4, 0)",
    )
    .bind(PRIVATE)
    .bind(GUILD)
    .bind(INSIDER_ROLE)
    .bind(VIEW_CHANNEL)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
         SELECT $1, user_id, 'guild', false, 0, ARRAY[]::text[]
           FROM unnest($2::bigint[]) AS user_id",
    )
    .bind(GUILD)
    .bind(vec![VIEWER, INSIDER])
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(INSIDER)
        .bind(INSIDER_ROLE)
        .execute(pool)
        .await?;
    support::complete_rosters_from_user_guilds(pool).await?;
    Ok(())
}

async fn insert_session(pool: &PgPool, starting: i64, fragments: &[i64]) -> sqlx::Result<i64> {
    let session: i64 = sqlx::query_scalar(
        "INSERT INTO recording_sessions
            (guild_id, user_id, starting_channel_id, current_channel_id, state, started_at)
         VALUES ($1, $2, $3, $3, 'active', now())
         RETURNING id",
    )
    .bind(GUILD)
    .bind(OWNER)
    .bind(starting)
    .fetch_one(pool)
    .await?;
    for (index, channel) in fragments.iter().enumerate() {
        add_fragment(
            pool,
            session,
            *channel,
            i32::try_from(index).unwrap_or_default(),
        )
        .await?;
    }
    Ok(session)
}

async fn add_fragment(pool: &PgPool, session: i64, channel: i64, index: i32) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO audio_files
            (file_name, guild_id, channel_id, user_id, year, month, start_ts,
             recording_session_id, segment_index)
         VALUES ($1, $2, $3, $4, 2026, 10, 1000, $5, $6)",
    )
    .bind(format!("s{session}-f{index}"))
    .bind(GUILD)
    .bind(channel)
    .bind(OWNER)
    .bind(session)
    .bind(index)
    .execute(pool)
    .await?;
    Ok(())
}

fn server(pool: &PgPool, hub: web::Data<Hub>, realtime_enabled: bool) -> actix_test::TestServer {
    let pool = pool.clone();
    actix_test::start(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(keys()))
            .app_data(web::Data::new(config(realtime_enabled)))
            .app_data(hub.clone())
            .service(
                web::scope("/api")
                    .wrap(AuthMiddleware)
                    .service(realtime_socket),
            )
    })
}

fn cookie(user_id: i64) -> Result<String, Box<dyn std::error::Error>> {
    let token = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
        "csrf".into(),
        &EncodingKey::from_secret(b"test_secret"),
    )?;
    Ok(format!("{ACCESS_TOKEN_COOKIE}={token}"))
}

async fn connect(
    server: &actix_test::TestServer,
    user_id: i64,
    origin: &str,
) -> Result<Result<Socket, u16>, Box<dyn std::error::Error>> {
    let request = awc::Client::new()
        .ws(server.url("/api/realtime"))
        .origin(origin)
        .set_header("Cookie", cookie(user_id)?);
    match request.connect().await {
        Ok((_, socket)) => Ok(Ok(socket)),
        Err(awc::error::WsClientError::InvalidResponseStatus(status)) => Ok(Err(status.as_u16())),
        Err(error) => Err(error.to_string().into()),
    }
}

/// The next server message, skipping heartbeats; `Err` names a close code.
async fn next(
    socket: &mut Socket,
) -> Result<Result<Value, Option<u16>>, Box<dyn std::error::Error>> {
    loop {
        let frame = tokio::time::timeout(WAIT, socket.next())
            .await?
            .ok_or("socket ended")??;
        match frame {
            Frame::Text(text) => {
                let message: Value = serde_json::from_slice(&text)?;
                if message["type"] != "heartbeat" {
                    return Ok(Ok(message));
                }
            }
            Frame::Close(reason) => return Ok(Err(reason.map(|reason| u16::from(reason.code)))),
            _ => {}
        }
    }
}

async fn expect(socket: &mut Socket) -> Result<Value, Box<dyn std::error::Error>> {
    next(socket)
        .await?
        .map_err(|code| format!("closed with {code:?}").into())
}

async fn subscribe(socket: &mut Socket) -> TestResult {
    assert_eq!(expect(socket).await?["type"], "ready");
    socket
        .send(Message::Text(
            json!({ "type": "set_scope", "v": 1, "guild_id": GUILD.to_string() })
                .to_string()
                .into(),
        ))
        .await?;
    assert_eq!(
        expect(socket).await?,
        json!({ "type": "subscribed", "v": 1, "guild_id": GUILD.to_string() })
    );
    Ok(())
}

fn changed(resource: &str, ids: Option<&[i64]>) -> Value {
    let mut message = json!({
        "type": "changed", "v": 1, "guild_id": GUILD.to_string(), "resource": resource,
    });
    if let Some(ids) = ids {
        message["ids"] = json!(ids.iter().map(i64::to_string).collect::<Vec<_>>());
    }
    message
}

/// The listener connects lazily; write until the first event comes through.
async fn wait_for_listener(pool: &PgPool, socket: &mut Socket) -> TestResult {
    for _ in 0..20 {
        let session = insert_session(pool, PUBLIC, &[]).await?;
        let received = tokio::time::timeout(Duration::from_millis(400), next(socket)).await;
        if let Ok(Ok(Ok(message))) = received
            && message == changed("recordings", Some(&[session]))
        {
            return Ok(());
        }
    }
    Err("realtime listener never delivered".into())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn viewers_receive_only_what_they_may_list(pool: PgPool) -> TestResult {
    // awc and actix-test spawn local tasks.
    tokio::task::LocalSet::new()
        .run_until(viewers_receive_only_what_they_may_list_body(pool))
        .await
}

async fn viewers_receive_only_what_they_may_list_body(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let hub = web::Data::new(Hub::new(pool.clone()));
    let tasks = web_server::realtime::spawn_listener(&pool, hub.clone().into_inner());
    let server = server(&pool, hub.clone(), true);

    let mut viewer = connect(&server, VIEWER, ORIGIN)
        .await?
        .map_err(|s| format!("{s}"))?;
    let mut insider = connect(&server, INSIDER, ORIGIN)
        .await?
        .map_err(|s| format!("{s}"))?;
    subscribe(&mut viewer).await?;
    subscribe(&mut insider).await?;
    wait_for_listener(&pool, &mut viewer).await?;
    // The insider saw the warm-up sessions too; skip to a known point.
    let marker = insert_session(&pool, PUBLIC, &[]).await?;
    while expect(&mut insider).await? != changed("recordings", Some(&[marker])) {}
    assert_eq!(
        expect(&mut viewer).await?,
        changed("recordings", Some(&[marker]))
    );

    // A private session: its id reaches only the insider, and the viewer is
    // not told anything happened.
    let private = insert_session(&pool, PRIVATE, &[PRIVATE]).await?;
    assert_eq!(
        expect(&mut insider).await?,
        changed("recordings", Some(&[private]))
    );

    // A public session that moves into the private channel: the insider gets
    // its id; the viewer, who listed it before, gets an id-free refresh.
    let moving = insert_session(&pool, PUBLIC, &[PUBLIC]).await?;
    assert_eq!(
        expect(&mut insider).await?,
        changed("recordings", Some(&[moving]))
    );
    assert_eq!(
        expect(&mut viewer).await?,
        changed("recordings", Some(&[moving]))
    );
    add_fragment(&pool, moving, PRIVATE, 1).await?;
    assert_eq!(
        expect(&mut insider).await?,
        changed("recordings", Some(&[moving]))
    );
    assert_eq!(expect(&mut viewer).await?, changed("recordings", None));

    // Granting the viewer the role re-authorizes them before any later event.
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(VIEWER)
        .bind(INSIDER_ROLE)
        .execute(&pool)
        .await?;
    let access_changed = json!({ "type": "access_changed", "v": 1, "guild_id": GUILD.to_string() });
    assert_eq!(expect(&mut viewer).await?, access_changed);
    assert_eq!(expect(&mut insider).await?, access_changed);
    let now_visible = insert_session(&pool, PRIVATE, &[PRIVATE]).await?;
    assert_eq!(
        expect(&mut viewer).await?,
        changed("recordings", Some(&[now_visible]))
    );

    // Shutdown: every socket is closed at once with 1012.
    hub.close_all(web_server::realtime::protocol::CLOSE_SERVICE_RESTART);
    for socket in [&mut viewer, &mut insider] {
        loop {
            match next(socket).await? {
                Ok(_) => continue,
                Err(code) => {
                    assert_eq!(code, Some(1012));
                    break;
                }
            }
        }
    }
    for task in tasks {
        task.abort();
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn refused_connections_and_messages(pool: PgPool) -> TestResult {
    // awc and actix-test spawn local tasks.
    tokio::task::LocalSet::new()
        .run_until(refused_connections_and_messages_body(pool))
        .await
}

async fn refused_connections_and_messages_body(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let hub = web::Data::new(Hub::new(pool.clone()));

    // Realtime off: the endpoint does not exist.
    let off = server(&pool, hub.clone(), false);
    assert_eq!(connect(&off, VIEWER, ORIGIN).await?.err(), Some(404));

    let on = server(&pool, hub.clone(), true);
    // Another site, or a subdomain the CORS rule would accept: refused.
    for origin in ["https://evil.example", "https://x.app.example.com", "null"] {
        assert_eq!(
            connect(&on, VIEWER, origin).await?.err(),
            Some(403),
            "{origin}"
        );
    }

    // A role preview by a non-manager is refused with access_changed.
    let mut socket = connect(&on, VIEWER, ORIGIN)
        .await?
        .map_err(|s| format!("{s}"))?;
    assert_eq!(expect(&mut socket).await?["type"], "ready");
    socket
        .send(Message::Text(
            json!({
                "type": "set_scope", "v": 1,
                "guild_id": GUILD.to_string(), "as_role": INSIDER_ROLE.to_string(),
            })
            .to_string()
            .into(),
        ))
        .await?;
    assert_eq!(
        expect(&mut socket).await?,
        json!({ "type": "access_changed", "v": 1, "guild_id": GUILD.to_string() })
    );

    // An unsupported protocol version closes with 4400.
    socket
        .send(Message::Text(
            json!({ "type": "heartbeat", "v": 2 }).to_string().into(),
        ))
        .await?;
    assert_eq!(next(&mut socket).await?, Err(Some(4400)));
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_lost_listener_connection_resyncs_every_socket(pool: PgPool) -> TestResult {
    // awc and actix-test spawn local tasks.
    tokio::task::LocalSet::new()
        .run_until(a_lost_listener_connection_resyncs_every_socket_body(pool))
        .await
}

async fn a_lost_listener_connection_resyncs_every_socket_body(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let hub = web::Data::new(Hub::new(pool.clone()));
    let tasks = web_server::realtime::spawn_listener(&pool, hub.clone().into_inner());
    let server = server(&pool, hub.clone(), true);
    let mut socket = connect(&server, VIEWER, ORIGIN)
        .await?
        .map_err(|s| format!("{s}"))?;
    subscribe(&mut socket).await?;
    wait_for_listener(&pool, &mut socket).await?;

    // Kill the listener's database session: notifications sent while it is
    // gone are lost, so every socket must refetch once LISTEN is back.
    let killed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (
             SELECT pg_terminate_backend(pid) FROM pg_stat_activity
              WHERE query ILIKE 'LISTEN%' AND datname = current_database()
         ) AS terminated",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(killed, 1);

    loop {
        let message = expect(&mut socket).await?;
        if message["type"] == "resync_required" {
            assert_eq!(message["reason"], "listener_reconnected");
            break;
        }
    }
    // And delivery resumes.
    let session = insert_session(&pool, PUBLIC, &[]).await?;
    loop {
        if expect(&mut socket).await? == changed("recordings", Some(&[session])) {
            break;
        }
    }
    for task in tasks {
        task.abort();
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn leaving_and_joining_the_guild_follow_the_roster(pool: PgPool) -> TestResult {
    tokio::task::LocalSet::new()
        .run_until(leaving_and_joining_the_guild_follow_the_roster_body(pool))
        .await
}

async fn leaving_and_joining_the_guild_follow_the_roster_body(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let hub = web::Data::new(Hub::new(pool.clone()));
    let _tasks = web_server::realtime::spawn_listener(&pool, hub.clone().into_inner());
    let server = server(&pool, hub.clone(), true);
    let mut viewer = connect(&server, VIEWER, ORIGIN)
        .await?
        .map_err(|s| format!("{s}"))?;
    subscribe(&mut viewer).await?;
    wait_for_listener(&pool, &mut viewer).await?;

    // Leaving the guild (a member-remove event) revokes access at once, even
    // though the login snapshot still lists the guild.
    support::leave_guild(&pool, GUILD, VIEWER).await?;
    let mut message = expect(&mut viewer).await?;
    while message["type"] == "changed" {
        message = expect(&mut viewer).await?;
    }
    assert_eq!(
        message,
        json!({ "type": "access_changed", "v": 1, "guild_id": GUILD.to_string() })
    );
    // Nothing reaches a viewer who is out of the guild.
    insert_session(&pool, PUBLIC, &[]).await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(500), next(&mut viewer))
            .await
            .is_err()
    );

    // Joining again subscribes the scope the client asked for, with no new
    // `set_scope`; events flow again after it.
    sqlx::query("INSERT INTO guild_members (guild_id, user_id, username) VALUES ($1, $2, 'back')")
        .bind(GUILD)
        .bind(VIEWER)
        .execute(&pool)
        .await?;
    assert_eq!(
        expect(&mut viewer).await?,
        json!({ "type": "subscribed", "v": 1, "guild_id": GUILD.to_string() })
    );
    // The guild list changed too.
    assert_eq!(
        expect(&mut viewer).await?,
        json!({ "type": "access_changed", "v": 1, "guild_id": GUILD.to_string() })
    );
    let visible = insert_session(&pool, PUBLIC, &[]).await?;
    assert_eq!(
        expect(&mut viewer).await?,
        changed("recordings", Some(&[visible]))
    );
    Ok(())
}

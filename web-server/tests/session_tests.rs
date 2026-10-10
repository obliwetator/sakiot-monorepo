//! Sign-in sessions: a refresh succeeds only while its session row exists,
//! logout ends the asking browser's session, and "sign out everywhere" ends
//! every session of one identity.

use actix_web::{App, cookie::Cookie, http::StatusCode, test, web};
use jsonwebtoken::{DecodingKey, EncodingKey, Header};
use sqlx::PgPool;
use web_server::auth::cookies::{
    ACCESS_TOKEN_COOKIE, CSRF_COOKIE, LOGGED_IN_COOKIE, REFRESH_TOKEN_COOKIE,
};
use web_server::auth::device::Device;
use web_server::auth::{
    Access, AccessKeys, AuthKind, AuthMiddleware, Refresh, Token, end_all_sessions, end_session,
    list_sessions, logout, refresh_jwt, sessions,
};

const CSRF: &str = "csrf-123";

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn keys() -> AccessKeys {
    AccessKeys {
        access_encode: EncodingKey::from_secret(b"access"),
        refresh_encode: EncodingKey::from_secret(b"refresh"),
        access_decode: DecodingKey::from_secret(b"access"),
        refresh_decode: DecodingKey::from_secret(b"refresh"),
    }
}

/// An access and a refresh token for `session_id`, as sign-in issues them.
fn tokens(
    user_id: i64,
    auth_kind: AuthKind,
    session_id: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let keys = keys();
    Ok((
        Token::<Access>::encode(
            user_id,
            auth_kind,
            session_id.into(),
            CSRF.into(),
            &keys.access_encode,
        )?,
        Token::<Refresh>::encode(
            user_id,
            auth_kind,
            session_id.into(),
            CSRF.into(),
            &keys.refresh_encode,
        )?,
    ))
}

/// A signed-in session: its row and the refresh token naming it.
async fn sign_in(
    pool: &PgPool,
    user_id: i64,
    auth_kind: AuthKind,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let session_id = sessions::start(pool, user_id, auth_kind, Device::default()).await?;
    tokens(user_id, auth_kind, &session_id)
}

macro_rules! app {
    ($pool:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new(keys()))
                .service(
                    web::scope("/api")
                        .wrap(AuthMiddleware)
                        .service(refresh_jwt)
                        .service(logout)
                        .service(list_sessions)
                        .service(end_all_sessions)
                        .service(end_session),
                ),
        )
        .await
    };
}

fn refresh_request(refresh_token: &str) -> test::TestRequest {
    test::TestRequest::post()
        .uri("/api/refresh")
        .cookie(Cookie::new(REFRESH_TOKEN_COOKIE, refresh_token.to_owned()))
        .cookie(Cookie::new(CSRF_COOKIE, CSRF))
        .insert_header(("X-CSRF-Token", CSRF))
}

fn cleared_cookies(response: &actix_web::dev::ServiceResponse) -> Vec<String> {
    response
        .response()
        .cookies()
        .filter(|cookie| cookie.value().is_empty())
        .map(|cookie| cookie.name().to_owned())
        .collect()
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn refresh_failures_follow_the_error_contract(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (_, refresh) = sign_in(&pool, 7, AuthKind::Discord).await?;

    // No refresh token, or one that does not verify: the session is over.
    for request in [
        test::TestRequest::post().uri("/api/refresh"),
        test::TestRequest::post()
            .uri("/api/refresh")
            .cookie(Cookie::new(REFRESH_TOKEN_COOKIE, "not-a-token")),
    ] {
        let response = test::call_service(&app, request.to_request()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let cleared = cleared_cookies(&response);
        assert!(cleared.iter().any(|name| name == REFRESH_TOKEN_COOKIE));
        assert!(cleared.iter().any(|name| name == LOGGED_IN_COOKIE));
        let body: serde_json::Value = test::read_body_json(response).await;
        assert_eq!(body["code"], 401);
        assert_eq!(body["kind"], "unauthorized");
    }

    // A valid refresh token with a stale or missing CSRF token.
    for csrf_header in [Some("csrf-stale"), None] {
        let mut request = test::TestRequest::post()
            .uri("/api/refresh")
            .cookie(Cookie::new(REFRESH_TOKEN_COOKIE, refresh.clone()))
            .cookie(Cookie::new(CSRF_COOKIE, CSRF));
        if let Some(csrf) = csrf_header {
            request = request.insert_header(("X-CSRF-Token", csrf));
        }
        let response = test::call_service(&app, request.to_request()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: serde_json::Value = test::read_body_json(response).await;
        assert_eq!(body["kind"], "csrf_rejected");
    }

    // A successful refresh issues new tokens but keeps the login's CSRF.
    let response = test::call_service(&app, refresh_request(&refresh).to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok()),
        Some(CSRF)
    );
    let cookies: Vec<(String, String)> = response
        .response()
        .cookies()
        .map(|cookie| (cookie.name().to_owned(), cookie.value().to_owned()))
        .collect();
    assert!(cookies.contains(&(CSRF_COOKIE.to_owned(), CSRF.to_owned())));
    let renewed = cookies
        .iter()
        .find(|(name, value)| name == REFRESH_TOKEN_COOKIE && !value.is_empty())
        .map(|(_, value)| value.clone())
        .ok_or("refresh cookie missing")?;

    // The renewed token names the same session and keeps refreshing.
    assert_eq!(
        Token::<Refresh>::decode(&renewed, &keys())?.session_id,
        Token::<Refresh>::decode(&refresh, &keys())?.session_id
    );
    let response = test::call_service(&app, refresh_request(&renewed).to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn refresh_needs_a_live_session(pool: PgPool) -> TestResult {
    let app = app!(pool);

    // Correctly signed, but no such session.
    let (_, unknown) = tokens(7, AuthKind::Discord, "never-started")?;
    let response = test::call_service(&app, refresh_request(&unknown).to_request()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        cleared_cookies(&response)
            .iter()
            .any(|name| name == REFRESH_TOKEN_COOKIE)
    );

    // The session belongs to another identity.
    let session_id = sessions::start(&pool, 8, AuthKind::Discord, Device::default()).await?;
    for (user_id, auth_kind) in [(7, AuthKind::Discord), (8, AuthKind::Dev)] {
        let (_, forged) = tokens(user_id, auth_kind, &session_id)?;
        let response = test::call_service(&app, refresh_request(&forged).to_request()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // A token issued before sessions existed has no session id.
    #[derive(serde::Serialize)]
    struct LegacyRefresh {
        exp: i64,
        user_id: i64,
        csrf: &'static str,
        auth_kind: &'static str,
        purpose: &'static str,
    }
    let legacy = jsonwebtoken::encode(
        &Header::default(),
        &LegacyRefresh {
            exp: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            user_id: 7,
            csrf: CSRF,
            auth_kind: "discord",
            purpose: "refresh",
        },
        &keys().refresh_encode,
    )?;
    let response = test::call_service(&app, refresh_request(&legacy).to_request()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn logout_ends_only_this_browsers_session(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (_, this_browser) = sign_in(&pool, 7, AuthKind::Discord).await?;
    let (_, other_browser) = sign_in(&pool, 7, AuthKind::Discord).await?;

    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/logout")
            .cookie(Cookie::new(REFRESH_TOKEN_COOKIE, this_browser.clone()))
            .cookie(Cookie::new(CSRF_COOKIE, CSRF))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        cleared_cookies(&response)
            .iter()
            .any(|name| name == REFRESH_TOKEN_COOKIE)
    );

    let response = test::call_service(&app, refresh_request(&this_browser).to_request()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = test::call_service(&app, refresh_request(&other_browser).to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn signing_out_everywhere_ends_every_session_of_that_identity(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (access, phone) = sign_in(&pool, 7, AuthKind::Discord).await?;
    let (_, laptop) = sign_in(&pool, 7, AuthKind::Discord).await?;
    // A dev login with the same id and another member are other identities.
    let (_, dev_login) = sign_in(&pool, 7, AuthKind::Dev).await?;
    let (_, other_member) = sign_in(&pool, 8, AuthKind::Discord).await?;

    // Like every other write, it needs the CSRF header.
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/api/users/current/sessions")
            .cookie(Cookie::new(ACCESS_TOKEN_COOKIE, access.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/api/users/current/sessions")
            .cookie(Cookie::new(ACCESS_TOKEN_COOKIE, access))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        cleared_cookies(&response)
            .iter()
            .any(|name| name == ACCESS_TOKEN_COOKIE)
    );

    for ended in [&phone, &laptop] {
        let response = test::call_service(&app, refresh_request(ended).to_request()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    for kept in [&dev_login, &other_member] {
        let response = test::call_service(&app, refresh_request(kept).to_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn sign_in_sweeps_sessions_that_can_no_longer_refresh(pool: PgPool) -> TestResult {
    let stale = sessions::start(&pool, 7, AuthKind::Discord, Device::default()).await?;
    let recent = sessions::start(&pool, 8, AuthKind::Discord, Device::default()).await?;
    sqlx::query!(
        "UPDATE auth_sessions SET refreshed_at = now() - interval '8 days' WHERE id = $1",
        stale,
    )
    .execute(&pool)
    .await?;
    sqlx::query!(
        "UPDATE auth_sessions SET refreshed_at = now() - interval '6 days' WHERE id = $1",
        recent,
    )
    .execute(&pool)
    .await?;

    let fresh = sessions::start(&pool, 9, AuthKind::Dev, Device::default()).await?;

    let mut remaining = sqlx::query_scalar!("SELECT id FROM auth_sessions")
        .fetch_all(&pool)
        .await?;
    remaining.sort();
    let mut expected = vec![recent, fresh];
    expected.sort();
    assert_eq!(remaining, expected);
    Ok(())
}

const CHROME_ON_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const CHROME_ON_ANDROID: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36";

/// Sign in as a browser with this User-Agent; returns the session id too.
async fn sign_in_from(
    pool: &PgPool,
    user_id: i64,
    user_agent: &str,
) -> Result<(String, String, String), Box<dyn std::error::Error>> {
    let session_id = sessions::start(
        pool,
        user_id,
        AuthKind::Discord,
        Device::from_user_agent(user_agent),
    )
    .await?;
    let (access, refresh) = tokens(user_id, AuthKind::Discord, &session_id)?;
    Ok((session_id, access, refresh))
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn the_devices_list_names_each_session_and_marks_this_one(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (desktop, access, _) = sign_in_from(&pool, 7, CHROME_ON_WINDOWS).await?;
    let (phone, _, _) = sign_in_from(&pool, 7, CHROME_ON_ANDROID).await?;
    // Another member's session and an unrecognized browser of this one's
    // dev identity are not this account's.
    sign_in_from(&pool, 8, CHROME_ON_WINDOWS).await?;
    sessions::start(&pool, 7, AuthKind::Dev, Device::default()).await?;
    sqlx::query!(
        "UPDATE auth_sessions SET refreshed_at = now() - interval '1 hour' WHERE id = $1",
        desktop,
    )
    .execute(&pool)
    .await?;

    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/users/current/sessions")
            .cookie(Cookie::new(ACCESS_TOKEN_COOKIE, access))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    let listed = body.as_array().ok_or("expected a list")?;
    assert_eq!(listed.len(), 2);

    // Most recently active first.
    assert_eq!(listed[0]["id"], phone.as_str());
    assert_eq!(listed[0]["browser"], "Chrome");
    assert_eq!(listed[0]["platform"], "Android");
    assert_eq!(listed[0]["current"], false);
    assert_eq!(listed[1]["id"], desktop.as_str());
    assert_eq!(listed[1]["platform"], "Windows");
    assert_eq!(listed[1]["current"], true);
    assert!(listed[1]["signed_in_at"].is_string());
    assert!(listed[1]["last_active_at"].is_string());
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn ending_one_session_signs_out_only_that_device(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (_, access, this_refresh) = sign_in_from(&pool, 7, CHROME_ON_WINDOWS).await?;
    let (phone, _, phone_refresh) = sign_in_from(&pool, 7, CHROME_ON_ANDROID).await?;
    let (other_member, _, _) = sign_in_from(&pool, 8, CHROME_ON_WINDOWS).await?;
    let end = |session_id: &str, csrf: bool| {
        let mut request = test::TestRequest::delete()
            .uri(&format!("/api/users/current/sessions/{session_id}"))
            .cookie(Cookie::new(ACCESS_TOKEN_COOKIE, access.clone()));
        if csrf {
            request = request.insert_header(("X-CSRF-Token", CSRF));
        }
        request.to_request()
    };

    let response = test::call_service(&app, end(&phone, false)).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Another member's session is not this account's to end.
    let response = test::call_service(&app, end(&other_member, true)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = test::call_service(&app, end(&phone, true)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(cleared_cookies(&response).is_empty());
    let response = test::call_service(&app, refresh_request(&phone_refresh).to_request()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = test::call_service(&app, refresh_request(&this_refresh).to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = test::call_service(&app, end(&phone, true)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn ending_this_session_from_the_list_clears_its_cookies(pool: PgPool) -> TestResult {
    let app = app!(pool);
    let (this_session, access, refresh) = sign_in_from(&pool, 7, CHROME_ON_WINDOWS).await?;

    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/api/users/current/sessions/{this_session}"))
            .cookie(Cookie::new(ACCESS_TOKEN_COOKIE, access))
            .insert_header(("X-CSRF-Token", CSRF))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        cleared_cookies(&response)
            .iter()
            .any(|name| name == REFRESH_TOKEN_COOKIE)
    );
    let response = test::call_service(&app, refresh_request(&refresh).to_request()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

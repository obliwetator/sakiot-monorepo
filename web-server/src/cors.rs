//! Which browser origins may call the API with the viewer's cookies.

use actix_cors::Cors;
use actix_web::web;

use crate::config::Config;

/// Only this deployment's own frontends, matched exactly (see
/// [`Config::is_frontend_origin`]). Credentials are allowed, so trusting a
/// subdomain would let any page on a sibling host read and change data as the
/// visiting user: the session cookies are same-site there, and every
/// authenticated response carries the CSRF token in an exposed header.
pub fn cors(config: web::Data<Config>) -> Cors {
    Cors::default()
        .allowed_origin_fn(move |origin, _req_head| {
            origin
                .to_str()
                .is_ok_and(|origin| config.is_frontend_origin(origin))
        })
        .allow_any_method()
        .allow_any_header()
        // Media element streaming needs these readable from JS / browser
        // internals; not safelisted by default under CORS.
        .expose_headers([
            "Content-Length",
            "Content-Range",
            "Content-Disposition",
            "ETag",
            "Accept-Ranges",
            "X-CSRF-Token",
        ])
        .supports_credentials()
        .max_age(3600)
}

#[cfg(test)]
mod tests {
    use super::cors;
    use crate::config::Config;
    use actix_web::http::{Method, StatusCode, header};
    use actix_web::{App, HttpResponse, test, web};

    fn config() -> Config {
        Config {
            database_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            access_secret: String::new(),
            refresh_secret: String::new(),
            dev_account_id: 0,
            dev_login_secret: None,
            cors_allowed_origin: "https://example.com".into(),
            oauth_allowed_opener_origins: vec!["https://login.example.com".into()],
            oauth_allowed_opener_host_suffixes: Vec::new(),
            discord_redirect_uri: String::new(),
            grpc_address: String::new(),
            fbi_agent_registry_secret: None,
            host: String::new(),
            port: 0,
            db_max_connections: 1,
            recording_permanent_delete_enabled: false,
            server_timing_header: false,
        }
    }

    #[actix_web::test]
    async fn only_exact_frontend_origins_get_credentialed_access() {
        let app = test::init_service(
            App::new()
                .route(
                    "/api/clips",
                    web::delete().to(|| async { HttpResponse::Ok().finish() }),
                )
                .wrap(cors(web::Data::new(config()))),
        )
        .await;

        for allowed in ["https://example.com", "https://login.example.com"] {
            let preflight = test::TestRequest::default()
                .method(Method::OPTIONS)
                .uri("/api/clips")
                .insert_header((header::ORIGIN, allowed))
                .insert_header((header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE"))
                .insert_header((header::ACCESS_CONTROL_REQUEST_HEADERS, "x-csrf-token"))
                .to_request();
            let response = test::call_service(&app, preflight).await;
            assert_eq!(response.status(), StatusCode::OK, "{allowed}");
            assert_eq!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                allowed
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                    .unwrap(),
                "true"
            );
        }

        for rejected in [
            "https://staging.example.com",
            "https://slot.preview.example.com",
            "https://example.com.evil.test",
            "http://example.com",
            "null",
        ] {
            let preflight = test::TestRequest::default()
                .method(Method::OPTIONS)
                .uri("/api/clips")
                .insert_header((header::ORIGIN, rejected))
                .insert_header((header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE"))
                .insert_header((header::ACCESS_CONTROL_REQUEST_HEADERS, "x-csrf-token"))
                .to_request();
            let response = test::call_service(&app, preflight).await;
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                "preflight from {rejected} was allowed"
            );

            // A simple request still reaches the handler (CORS is enforced by
            // the browser), but its response must stay unreadable.
            let simple = test::TestRequest::default()
                .method(Method::GET)
                .uri("/api/clips")
                .insert_header((header::ORIGIN, rejected))
                .to_request();
            let response = test::call_service(&app, simple).await;
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                "response to {rejected} was readable"
            );
        }
    }
}

use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "sakiot web_server",
        description = "HTTP + WS API for the sakiot Discord bot stack.",
        version = "0.1.0",
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "Discord OAuth, JWT refresh, logout"),
        (name = "user", description = "Current user + guild membership"),
        (name = "clips", description = "Voice clip CRUD and playback"),
    ),
    paths(
        crate::auth::handlers::logout,
        crate::user::get_current_user,
    ),
    components(schemas(crate::user::UserDataForFrontEnd)),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.as_mut().expect("components registered");
        components.add_security_scheme(
            "access_token",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("access_token"))),
        );
        components.add_security_scheme(
            "csrf_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-CSRF-Token"))),
        );
    }
}

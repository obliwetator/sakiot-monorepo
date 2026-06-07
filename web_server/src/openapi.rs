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
        (name = "audio", description = "Recording listing, processing, and event metadata"),
        (name = "clips", description = "Voice clip CRUD and playback"),
        (name = "stamps", description = "Guild stamp metadata"),
        (name = "admin", description = "Guild admin configuration"),
    ),
    paths(
        crate::admin::cooldowns::delete_user_override,
        crate::admin::cooldowns::get_guild_cooldown,
        crate::admin::cooldowns::list_user_overrides,
        crate::admin::cooldowns::set_guild_cooldown,
        crate::admin::cooldowns::set_user_override,
        crate::audio::events::get_recording_events,
        crate::audio::listing::get_current_month_permission,
        crate::audio::listing::get_live_stems,
        crate::audio::live::live_state,
        crate::audio::silence::remove_silence,
        crate::auth::handlers::refresh_jwt,
        crate::auth::handlers::logout,
        crate::clips::create_clip,
        crate::clips::delete,
        crate::clips::get_clips,
        crate::stamps::get_stamps,
        crate::user::get_current_user,
        crate::user::get_current_user_guilds,
    ),
    components(schemas(
        crate::admin::cooldowns::CooldownBody,
        crate::admin::cooldowns::GuildCooldown,
        crate::admin::cooldowns::UserOverride,
        crate::audio::events::VoiceEventDto,
        crate::audio::live::StateResponse,
        crate::audio::silence::RemoveSilenceResponse,
        crate::audio::types::Channels,
        crate::audio::types::Directories,
        crate::audio::types::File,
        crate::audio::types::StartEnd,
        crate::auth::handlers::RefreshTokenError,
        crate::auth::handlers::RefreshTokenResponse,
        crate::clips::ClipInfo,
        crate::clips::CreateClipResponse,
        crate::errors::ApiError,
        crate::stamps::StampInfo,
        crate::user::GuildDataForFrontEnd,
        crate::user::UserDataForFrontEnd,
    )),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
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

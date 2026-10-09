use serenity::model::prelude::GuildId;
use songbird::SongbirdKey;
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

use crate::commands::voice_controls::{ClipPlayer, PlayClipError, play_clip};

use super::FbiAgentGrpc;
use super::proto::jam_response::JamResponseEnum;
use super::proto::jammer_server::Jammer;
use super::proto::{JamData, JamResponse};

#[tonic::async_trait]
impl Jammer for FbiAgentGrpc {
    async fn jam_it(&self, request: Request<JamData>) -> Result<Response<JamResponse>, Status> {
        let data = request.into_inner();

        let guild_id = match u64::try_from(data.guild_id) {
            Ok(id) => GuildId::new(id),
            Err(_) => {
                warn!("Invalid guild id from jam request: {}", data.guild_id);
                return Err(Status::invalid_argument("guild_id must be non-negative"));
            }
        };

        let manager = {
            let data_guard = self.data_cache.data.read().await;
            data_guard.get::<SongbirdKey>().cloned()
        };
        let Some(manager) = manager else {
            error!("Songbird manager missing from typemap");
            return Err(Status::internal("Songbird manager missing from typemap"));
        };

        let response = |resp: JamResponseEnum,
                        cooldown_remaining_seconds: u32|
         -> Result<Response<JamResponse>, Status> {
            Ok(Response::new(JamResponse {
                resp: resp.into(),
                cooldown_remaining_seconds,
            }))
        };
        let player = ClipPlayer {
            pool: &self.data_cache.pool,
            media_archive: &self.data_cache.media_archive,
            manager: &manager,
            cache: &self.data_cache.cache,
            cooldown: &self.data_cache.jam_cooldown,
        };
        match play_clip(&player, guild_id, &data.clip_name, data.user_id).await {
            Ok(message) => {
                info!(guild_id = guild_id.get(), "{}", message);
                response(JamResponseEnum::Ok, 0)
            }
            Err(PlayClipError::NotInVoice) => response(JamResponseEnum::NotPresent, 0),
            Err(PlayClipError::Cooldown { remaining_secs }) => {
                response(JamResponseEnum::Cooldown, remaining_secs)
            }
            Err(PlayClipError::Db(db_err)) => {
                error!("Failed to handle gRPC jam playback: {}", db_err);
                Err(Status::internal(format!("database error: {db_err}")))
            }
            Err(err) => {
                error!("Failed to handle gRPC jam playback: {}", err);
                response(JamResponseEnum::Unknown, 0)
            }
        }
    }
}

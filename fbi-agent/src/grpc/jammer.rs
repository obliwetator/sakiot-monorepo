use std::sync::Arc;

use serenity::model::prelude::GuildId;
use songbird::{Songbird, SongbirdKey};
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

use crate::commands::voice_controls::PlayClipError;
use crate::cooldown::CheckResult;

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

        // Answer this from our own songbird, not from the guild cache: the bot user id
        // is shared with any draining instance, so seeing "the bot" in a voice channel
        // says nothing about whether *this* process holds that connection.
        if !holds_voice_connection(&manager, guild_id).await {
            return Ok(Response::new(JamResponse {
                resp: JamResponseEnum::NotPresent.into(),
                cooldown_remaining_seconds: 0,
            }));
        }

        // Only spend the cooldown once we know the clip can actually play.
        match self
            .data_cache
            .jam_cooldown
            .check_and_record(&self.data_cache.pool, data.guild_id, data.user_id)
            .await
        {
            Ok(CheckResult::Allowed) => {}
            Ok(CheckResult::OnCooldown { remaining_secs }) => {
                return Ok(Response::new(JamResponse {
                    resp: JamResponseEnum::Cooldown.into(),
                    cooldown_remaining_seconds: remaining_secs,
                }));
            }
            Err(err) => return Err(Status::internal(format!("database error: {err}"))),
        }

        match crate::commands::voice_controls::play_clip(
            &self.data_cache.pool,
            &self.data_cache.media_archive,
            &manager,
            guild_id,
            &data.clip_name,
            data.user_id,
        )
        .await
        {
            Ok(message) => {
                info!(guild_id = guild_id.get(), "{}", message);
                Ok(Response::new(JamResponse {
                    resp: JamResponseEnum::Ok.into(),
                    cooldown_remaining_seconds: 0,
                }))
            }
            Err(PlayClipError::Db(db_err)) => {
                error!("Failed to handle gRPC jam playback: {}", db_err);
                Err(Status::internal(format!("database error: {db_err}")))
            }
            // Lost the call between the check above and playback.
            Err(PlayClipError::NotInVoice) => Ok(Response::new(JamResponse {
                resp: JamResponseEnum::NotPresent.into(),
                cooldown_remaining_seconds: 0,
            })),
            Err(err) => {
                error!("Failed to handle gRPC jam playback: {}", err);
                Ok(Response::new(JamResponse {
                    resp: JamResponseEnum::Unknown.into(),
                    cooldown_remaining_seconds: 0,
                }))
            }
        }
    }
}

/// Whether this process is connected to voice in `guild_id`.
///
/// Mirrors `connected_voice_connection_count`: songbird keeps a `Call` around after a
/// disconnect, so a registered call is not on its own proof of presence.
async fn holds_voice_connection(manager: &Arc<Songbird>, guild_id: GuildId) -> bool {
    match manager.get(guild_id) {
        Some(call) => call.lock().await.current_channel().is_some(),
        None => false,
    }
}

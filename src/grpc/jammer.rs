use std::sync::Arc;

use serenity::model::prelude::GuildId;
use serenity::prelude::{RwLock, TypeMap};
use songbird::SongbirdKey;
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
        let application_id_release = crate::config::application_id_release()
            .map_err(|err| Status::internal(format!("invalid APPLICATION_ID_RELEASE: {err}")))?;

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

        let guild_id = match u64::try_from(data.guild_id) {
            Ok(id) => GuildId::new(id),
            Err(_) => {
                warn!("Invalid guild id from jam request: {}", data.guild_id);
                return Err(Status::invalid_argument("guild_id must be non-negative"));
            }
        };

        let guild = match self.data_cache.cache.guild(guild_id) {
            Some(g) => g.to_owned(),
            None => {
                return Ok(Response::new(JamResponse {
                    resp: JamResponseEnum::Unknown.into(),
                    cooldown_remaining_seconds: 0,
                }));
            }
        };

        for guild_channel in guild.channels.values() {
            if guild_channel.kind != serenity::model::prelude::ChannelType::Voice {
                continue;
            }
            let members = match guild_channel.members(&self.data_cache.cache) {
                Ok(members) => members,
                Err(err) => {
                    warn!(
                        channel_id = guild_channel.id.get(),
                        error = %err,
                        "failed to read channel members"
                    );
                    continue;
                }
            };

            for member in &members {
                if member.user.id == application_id_release {
                    info!(
                        "Ladies and gentlemen, We got him in c {}",
                        guild_channel.id.get()
                    );

                    if let Err(err) = handle_play_audio_to_channel(
                        data.guild_id,
                        &data.clip_name,
                        data.user_id,
                        self.data_cache.data.clone(),
                        self.data_cache.pool.clone(),
                    )
                    .await
                    {
                        error!("Failed to handle gRPC jam playback: {}", err);
                        if let PlayClipError::Db(db_err) = err {
                            return Err(Status::internal(format!("database error: {db_err}")));
                        }
                        return Ok(Response::new(JamResponse {
                            resp: JamResponseEnum::Unknown.into(),
                            cooldown_remaining_seconds: 0,
                        }));
                    }

                    return Ok(Response::new(JamResponse {
                        resp: JamResponseEnum::Ok.into(),
                        cooldown_remaining_seconds: 0,
                    }));
                }
            }
        }

        Ok(Response::new(JamResponse {
            resp: JamResponseEnum::NotPresent.into(),
            cooldown_remaining_seconds: 0,
        }))
    }
}

async fn handle_play_audio_to_channel(
    id: i64,
    clip_name: &str,
    user_id: i64,
    data: Arc<RwLock<TypeMap>>,
    pool: sqlx::Pool<sqlx::Postgres>,
) -> Result<(), PlayClipError> {
    let manager = {
        let data_guard = data.read().await;
        data_guard.get::<SongbirdKey>().cloned().ok_or_else(|| {
            PlayClipError::User("Songbird manager missing from typemap".to_string())
        })?
    };

    let guild_id = GuildId::new(
        u64::try_from(id)
            .map_err(|_| PlayClipError::User("guild_id must be non-negative".to_string()))?,
    );
    crate::commands::voice_controls::play_clip(&pool, &manager, guild_id, clip_name, user_id)
        .await
        .map(|_| ())
}

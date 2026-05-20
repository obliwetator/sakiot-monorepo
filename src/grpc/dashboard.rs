use std::sync::atomic::Ordering;
use std::time::Duration;

use serenity::model::prelude::GuildId;
use songbird::SongbirdKey;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::BotMetricsKey;

use super::MyJammer;
use super::hello_world::dashboard_server::Dashboard;
use super::hello_world::{
    ActionResponse, ClientMessage, DashboardEvent, Empty, GuildRequest, MetricsResponse,
};
use super::snapshot::{GlobalMetricsSnapshot, StreamLifetime};

#[tonic::async_trait]
impl Dashboard for MyJammer {
    type GetMetricsStream = ReceiverStream<Result<MetricsResponse, Status>>;

    async fn get_metrics(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<Self::GetMetricsStream>, Status> {
        let (tx, rx) = mpsc::channel(4);
        let data_cache = self.data_cache.clone();

        tokio::spawn(async move {
            let _lifetime = StreamLifetime::acquire(&data_cache.data).await;

            loop {
                let snap = {
                    let data_guard = data_cache.data.read().await;
                    GlobalMetricsSnapshot::capture(&data_guard, &data_cache.cache)
                };

                if tx.send(Ok(MetricsResponse::from(snap))).await.is_err() {
                    break;
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    type DashboardStreamStream = ReceiverStream<Result<DashboardEvent, Status>>;

    async fn dashboard_stream(
        &self,
        request: Request<tonic::Streaming<ClientMessage>>,
    ) -> Result<Response<Self::DashboardStreamStream>, Status> {
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(100);
        let data_cache = self.data_cache.clone();

        let (topic_tx, mut topic_rx) = tokio::sync::watch::channel(String::new());

        tokio::spawn(async move {
            while let Ok(Some(msg)) = stream.message().await {
                if msg.action == "subscribe" {
                    let _ = topic_tx.send(msg.topic.clone());
                    info!("Client subscribed to topic: {}", msg.topic);
                } else if msg.action == "unsubscribe" {
                    info!("Client unsubscribed from topic: {}", msg.topic);
                    let _ = topic_tx.send(String::new());
                }
            }
        });

        tokio::spawn(async move {
            let _lifetime = StreamLifetime::acquire(&data_cache.data).await;

            let (mut global_rx, mut voice_rx) = {
                let data_guard = data_cache.data.read().await;
                let Some(metrics) = data_guard.get::<BotMetricsKey>() else {
                    return;
                };
                (
                    metrics.update_tx.subscribe(),
                    metrics.voice_update_tx.subscribe(),
                )
            };

            // Mark initial watch values as seen so changed() only fires on actual updates.
            let _ = global_rx.borrow_and_update();
            let _ = voice_rx.borrow_and_update();

            loop {
                // Read topic BEFORE sending so we always push the current state on every
                // iteration — including on startup when the topic may already be set
                // (race between Task-1 processing the subscribe and this task's loop).
                let topic = topic_rx.borrow().clone();

                if topic == "global" {
                    let (snap, guilds) = {
                        let data_guard = data_cache.data.read().await;
                        let snap = GlobalMetricsSnapshot::capture(&data_guard, &data_cache.cache);
                        let mut guilds = Vec::new();
                        if data_guard.get::<SongbirdKey>().is_some() {
                            for guild_id in data_cache.cache.guilds() {
                                if let Some(guild) = data_cache.cache.guild(guild_id) {
                                    guilds.push(serde_json::json!({
                                        "id": guild_id.get().to_string(),
                                        "name": guild.name.clone(),
                                    }));
                                }
                            }
                        }
                        (snap, guilds)
                    };

                    let json_payload = serde_json::json!({
                        "total_guilds": snap.total_guilds,
                        "active_voice_connections": snap.active_voice_connections,
                        "uptime_seconds": snap.uptime_seconds,
                        "commands_executed": snap.commands_executed,
                        "guilds": guilds,
                        "active_recordings": snap.active_recordings,
                        "writer_setup_failures": snap.writer_setup_failures,
                        "audio_packets_received": snap.audio_packets_received,
                        "audio_packets_dropped": snap.audio_packets_dropped,
                        "gateway_reconnects": snap.gateway_reconnects,
                        "driver_reconnects": snap.driver_reconnects,
                        "voice_state_updates_received": snap.voice_state_updates_received,
                        "db_query_errors": snap.db_query_errors,
                        "db_insert_failures": snap.db_insert_failures,
                        "grpc_active_streams": snap.grpc_active_streams,
                        "process_rss_bytes": snap.process_rss_bytes,
                        "process_open_fds": snap.process_open_fds,
                        "tokio_active_tasks": snap.tokio_active_tasks,
                        "messages_received": snap.messages_received,
                        "last_voice_packet_time": snap.last_voice_packet_time,
                    });

                    let event = DashboardEvent {
                        event_type: "METRICS_UPDATE".to_string(),
                        json_payload: json_payload.to_string(),
                    };

                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                } else if let Some(guild_id_str) = topic.strip_prefix("guild_voice:")
                    && let Ok(guild_id) = guild_id_str.parse::<u64>()
                {
                    let mut voice_states_json = Vec::new();
                    let mut user_start_times_json = serde_json::Map::new();

                    let (user_start_times, guild_rec_metrics) = {
                        let data_guard = data_cache.data.read().await;
                        if let Some(metrics) = data_guard.get::<BotMetricsKey>() {
                            (
                                Some(metrics.user_start_times.clone()),
                                Some(metrics.guild_metrics(guild_id)),
                            )
                        } else {
                            (None, None)
                        }
                    };

                    if let Some(guild) = data_cache.cache.guild(GuildId::new(guild_id)) {
                        for (user_id, voice_state) in &guild.voice_states {
                            if let Some(channel_id) = voice_state.channel_id {
                                voice_states_json.push(serde_json::json!({
                                    "user_id": user_id.get().to_string(),
                                    "channel_id": channel_id.get().to_string(),
                                    "mute": voice_state.mute,
                                    "deaf": voice_state.deaf,
                                    "self_mute": voice_state.self_mute,
                                    "self_deaf": voice_state.self_deaf,
                                    "self_stream": voice_state.self_stream.unwrap_or(false),
                                    "self_video": voice_state.self_video,
                                    "suppress": voice_state.suppress,
                                }));

                                if let Some(times) = &user_start_times
                                    && let Some(time) = times.get(&user_id.get())
                                {
                                    user_start_times_json.insert(
                                        user_id.get().to_string(),
                                        serde_json::Value::Number((*time).into()),
                                    );
                                }
                            }
                        }
                    }

                    let recording_metrics_json = guild_rec_metrics.map(|m| {
                            serde_json::json!({
                                "active_recordings": m.active_recordings.load(Ordering::Relaxed),
                                "writer_setup_failures": m.writer_setup_failures.load(Ordering::Relaxed),
                                "audio_packets_received": m.audio_packets_received.load(Ordering::Relaxed),
                                "audio_packets_dropped": m.audio_packets_dropped.load(Ordering::Relaxed),
                                "last_voice_packet_time": m.last_voice_packet_time.load(Ordering::Relaxed),
                            })
                        });

                    let event = DashboardEvent {
                        event_type: "GUILD_VOICE_UPDATE".to_string(),
                        json_payload: serde_json::json!({
                            "voice_states": voice_states_json,
                            "user_start_times": user_start_times_json,
                            "recording_metrics": recording_metrics_json,
                        })
                        .to_string(),
                    };

                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }

                // Wait for the topic to change or a relevant metric update before sending
                // the next payload.
                tokio::select! {
                    result = topic_rx.changed() => {
                        if result.is_err() { break; }
                    }
                    result = global_rx.changed(), if topic == "global" => {
                        if result.is_err() { break; }
                    }
                    result = voice_rx.changed(), if topic.starts_with("guild_voice:") => {
                        if result.is_err() { break; }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        // Periodic push so the UI stays live even when no event fires.
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn disconnect_voice(
        &self,
        request: Request<GuildRequest>,
    ) -> Result<Response<ActionResponse>, Status> {
        let req = request.into_inner();
        let guild_id = GuildId::new(
            u64::try_from(req.guild_id)
                .map_err(|_| Status::invalid_argument("guild_id must be non-negative"))?,
        );

        let data_guard = self.data_cache.data.read().await;
        let Some(songbird) = data_guard.get::<SongbirdKey>() else {
            warn!("disconnect_voice requested but Songbird manager is missing");
            return Ok(Response::new(ActionResponse {
                success: false,
                message: "Voice system is not configured".to_string(),
            }));
        };
        let manager = songbird.clone();

        if manager.get(guild_id).is_some() {
            if let Err(e) = manager.remove(guild_id).await {
                return Ok(Response::new(ActionResponse {
                    success: false,
                    message: format!("Failed to disconnect: {}", e),
                }));
            }

            return Ok(Response::new(ActionResponse {
                success: true,
                message: "Successfully disconnected from voice".to_string(),
            }));
        }

        Ok(Response::new(ActionResponse {
            success: false,
            message: "Bot is not in a voice channel in this guild".to_string(),
        }))
    }
}

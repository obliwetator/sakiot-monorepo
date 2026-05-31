use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

use serenity::{all::ApplicationId, client::Cache, http::Http, prelude::*};
use songbird::{Config, SerenityInit, driver::DecodeMode};
use sqlx::postgres::PgPoolOptions;
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::{
    event_handler::Handler,
    grpc::{FbiAgentGrpc, proto::jammer_server::JammerServer},
};

const SHARD_SHUTDOWN_TIMEOUT_SECONDS: u64 = 15;

pub mod metrics;
pub mod reaper;
pub use metrics::*;

pub mod commands;
pub mod config;
pub mod cooldown;
mod database;
pub mod deployment;
pub mod event_handler;
pub mod events;
pub mod grpc;
pub mod runtime;
pub mod telemetry;

#[cfg(test)]
mod tests;

pub struct HasBossMusic;
impl TypeMapKey for HasBossMusic {
    type Value = HashMap<u64, Option<String>>;
}

pub struct HelperStruct;
impl TypeMapKey for HelperStruct {
    type Value = Arc<RwLock<HashMap<u64, Option<u64>>>>;
}

#[derive(Clone)]
pub struct Custom {
    cache: Arc<Cache>,
    _http: Arc<Http>,
    data: Arc<RwLock<TypeMap>>,
    pub pool: sqlx::Pool<sqlx::Postgres>,
    pub jam_cooldown: crate::cooldown::JamCooldown,
    pub runtime: Arc<crate::runtime::RuntimeState>,
}

pub async fn get_lock_read(ctx: &Context) -> Arc<RwLock<HashMap<u64, Option<u64>>>> {
    if let Some(lock) = {
        let data_write = ctx.data.read().await;
        data_write.get::<HelperStruct>().cloned()
    } {
        return lock;
    }

    error!("HelperStruct missing from typemap; recreating it");
    let lock = Arc::new(RwLock::new(HashMap::new()));
    let mut data_write = ctx.data.write().await;
    data_write.insert::<HelperStruct>(lock.clone());
    lock
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenvy::dotenv().ok();

    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "Failed to install rustls crypto provider")?;

    crate::telemetry::init_telemetry()?;

    let recording_path = events::voice_receiver::recording_file_path();
    if !recording_path.exists() {
        tokio::fs::create_dir_all(&recording_path).await?;
    }

    let db_url = config::db_url()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    reaper::reap_zombie_recordings(&pool).await;

    // let a = conn.exec_map("SELECT * FROM guilds WHERE id IN (:id)", db_param, | id | DBGuild { id });
    // Configure the client with your Discord bot token in the environment.
    let discord_config = config::discord_config()?;
    let runtime = crate::runtime::RuntimeState::new(crate::runtime::RuntimeConfig::from_env());
    deployment::upsert_instance(&pool, &runtime).await;
    deployment::start_heartbeat(pool.clone(), runtime.clone());
    info!(
        instance_id = %runtime.config().instance_id,
        role = runtime.role().as_str(),
        drain_timeout_seconds = runtime.config().drain_timeout.as_secs(),
        "runtime configured"
    );

    // Here, we need to configure Songbird to decode all incoming voice packets.
    // If you want, you can do this on a per-call basis---here, we need it to
    // read the audio data that other people are sending us!
    let songbird_config = Config::default()
        .decode_mode(DecodeMode::Decode(songbird::driver::DecodeConfig::default()));

    let intents = GatewayIntents::all();
    // Create a new instance of the Client, logging in as a bot. This will
    // automatically prepend your bot token with "Bot ", which is a requirement
    // by Discord for bot users.
    let jam_cooldown = crate::cooldown::JamCooldown::new();

    let mut client = Client::builder(discord_config.token, intents)
        .event_handler(Handler {
            database: pool.clone(),
            jam_cooldown: jam_cooldown.clone(),
            runtime: runtime.clone(),
        })
        .intents(intents)
        .register_songbird_from_config(songbird_config)
        .application_id(ApplicationId::new(discord_config.application_id))
        .await?;
    {
        let mut data = client.data.write().await;
        // data.insert::<MysqlConnection>(mysql_pool.clone());
        data.insert::<HelperStruct>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<HasBossMusic>(HashMap::new());
        data.insert::<BotMetricsKey>(Arc::new(BotMetrics::default()));
        data.insert::<crate::runtime::RuntimeStateKey>(runtime.clone());
    }

    let http = client.http.clone();
    let cache = client.cache.clone();
    let data = client.data.clone();
    let shutdown_data = data.clone();

    let custom = Custom {
        cache,
        _http: http,
        data,
        pool: pool.clone(),
        jam_cooldown: jam_cooldown.clone(),
        runtime: runtime.clone(),
    };

    // Grab the metrics Arc before moving `client` into the spawn below.
    let process_metrics = {
        let data_read = client.data.read().await;
        data_read.get::<BotMetricsKey>().cloned()
    };
    let Some(process_metrics) = process_metrics else {
        return Err("BotMetrics not inserted".into());
    };

    let shard_manager = client.shard_manager.clone();
    let bot = tokio::spawn(async move {
        if let Err(err) = client.start().await {
            error!("Discord client exited with error: {}", err);
        }
    });
    let bot_abort = bot.abort_handle();

    // Background task: sample process health every 15 seconds.
    BotMetrics::start_sysinfo_monitoring(process_metrics.clone());

    // Register OpenTelemetry metrics.
    BotMetrics::register_otel_metrics(process_metrics, runtime.clone());

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let grpc_shutdown_rx = shutdown_rx.clone();

    let grpc_server = tokio::spawn(async move {
        let addr = match crate::config::grpc_addr().parse() {
            Ok(addr) => addr,
            Err(err) => {
                error!("Invalid gRPC address: {}", err);
                return;
            }
        };

        let jammer = FbiAgentGrpc::new(custom.clone());

        info!("gRPC server listening on {}", addr);

        Server::builder()
            .add_service(JammerServer::new(jammer.clone()))
            .add_service(crate::grpc::proto::admin_server::AdminServer::new(
                jammer.clone(),
            ))
            .add_service(crate::grpc::proto::dashboard_server::DashboardServer::new(
                jammer,
            ))
            .serve_with_shutdown(addr, async move {
                let mut rx = grpc_shutdown_rx;
                while !*rx.borrow() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .unwrap_or_else(|err| error!("gRPC server failed: {}", err));
    });

    let shutdown_runtime = runtime.clone();
    let shutdown_pool = pool.clone();
    let shutdown_task = tokio::spawn(async move {
        if !shutdown_runtime.shutdown_when_empty() {
            wait_for_shutdown_signal_or_drain(shutdown_runtime.clone()).await;
        }
        shutdown_runtime.start_drain(true);
        deployment::heartbeat_instance_and_leases(&shutdown_pool, &shutdown_runtime).await;

        let deadline = if shutdown_runtime.config().drain_timeout.is_zero() {
            None
        } else {
            Some(tokio::time::Instant::now() + shutdown_runtime.config().drain_timeout)
        };
        loop {
            if shutdown_runtime.force_shutdown_requested() {
                warn!("force shutdown requested; bypassing drain wait");
                break;
            }

            let active = active_voice_connection_count(&shutdown_data).await;
            if active == 0 {
                info!("drain complete; shutting down shards");
                break;
            }
            if let Some(deadline) = deadline
                && tokio::time::Instant::now() >= deadline
            {
                warn!(active_voice_connections = active, "drain timeout reached");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        let shutdown_result = tokio::time::timeout(
            Duration::from_secs(SHARD_SHUTDOWN_TIMEOUT_SECONDS),
            shard_manager.shutdown_all(),
        )
        .await;
        let bot_aborted_for_shutdown = if shutdown_result.is_err() {
            warn!(
                timeout_seconds = SHARD_SHUTDOWN_TIMEOUT_SECONDS,
                "shard shutdown timed out; aborting Discord client task"
            );
            bot_abort.abort();
            true
        } else {
            false
        };
        let _ = shutdown_tx.send(true);
        bot_aborted_for_shutdown
    });

    let (bot_result, grpc_result, shutdown_result) = tokio::join!(bot, grpc_server, shutdown_task);
    if let Err(err) = grpc_result {
        error!("gRPC task join error: {}", err);
    }
    let bot_aborted_for_shutdown = match shutdown_result {
        Ok(aborted) => aborted,
        Err(err) => {
            error!("shutdown task join error: {}", err);
            false
        }
    };
    if let Err(err) = bot_result {
        if bot_aborted_for_shutdown && err.is_cancelled() {
            warn!("Discord task aborted after shard shutdown timeout");
        } else {
            error!("Discord task join error: {}", err);
        }
    }

    deployment::mark_instance_stopped(&pool, &runtime).await;

    Ok(())
}

async fn active_voice_connection_count(data: &Arc<RwLock<TypeMap>>) -> u32 {
    let data_read = data.read().await;
    data_read
        .get::<songbird::SongbirdKey>()
        .map(|manager| manager.iter().count() as u32)
        .or_else(|| {
            data_read.get::<BotMetricsKey>().map(|metrics| {
                metrics
                    .active_voice_connections
                    .load(std::sync::atomic::Ordering::Relaxed)
            })
        })
        .unwrap_or(0)
}

async fn wait_for_shutdown_signal_or_drain(runtime: Arc<crate::runtime::RuntimeState>) {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    error!("failed to register SIGTERM handler: {}", err);
                    runtime.changed().await;
                    return;
                }
            };

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = sigterm.recv() => break,
                _ = runtime.changed() => {
                    if runtime.shutdown_when_empty() {
                        break;
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = runtime.changed() => {
                    if runtime.shutdown_when_empty() {
                        break;
                    }
                }
            }
        }
    }
}

use std::{collections::HashMap, error::Error, sync::Arc};

use serenity::{all::ApplicationId, prelude::*};
use songbird::{Config, SerenityInit, driver::DecodeMode};
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tracing::{error, info};

use crate::{BotMetrics, BotMetricsKey, Custom, HasBossMusic, deployment, event_handler::Handler};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub(crate) async fn run() -> AppResult<()> {
    dotenvy::dotenv().ok();
    install_crypto_provider()?;
    crate::telemetry::init_telemetry()?;
    ensure_recording_dir().await?;

    let pool = connect_database().await?;
    crate::reaper::reap_zombie_recordings(&pool).await?;

    let runtime = crate::runtime::RuntimeState::new(crate::runtime::RuntimeConfig::from_env());
    deployment::upsert_instance(&pool, &runtime).await?;
    deployment::start_heartbeat(pool.clone(), runtime.clone());
    info!(
        instance_id = %runtime.config().instance_id,
        role = runtime.role().as_str(),
        drain_timeout_seconds = runtime.config().drain_timeout.as_secs(),
        "runtime configured"
    );

    let jam_cooldown = crate::cooldown::JamCooldown::new();
    let mut client = build_discord_client(&pool, runtime.clone(), jam_cooldown.clone()).await?;
    insert_typemap_state(&mut client, runtime.clone()).await;

    let custom = Custom::new(
        client.cache.clone(),
        client.http.clone(),
        client.data.clone(),
        pool.clone(),
        jam_cooldown,
        runtime.clone(),
    );
    let process_metrics = process_metrics(&client).await?;
    BotMetrics::start_sysinfo_monitoring(process_metrics.clone());
    BotMetrics::register_otel_metrics(process_metrics, runtime.clone());

    let shard_manager = client.shard_manager.clone();
    let shutdown_data = client.data.clone();
    let bot = tokio::spawn(async move {
        if let Err(err) = client.start().await {
            error!("Discord client exited with error: {}", err);
        }
    });
    let bot_abort = bot.abort_handle();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let grpc_server = crate::grpc::spawn_server(custom, shutdown_rx);
    let shutdown_task = crate::shutdown::spawn_shutdown_task(
        runtime.clone(),
        pool.clone(),
        shutdown_data,
        shard_manager,
        bot_abort,
        shutdown_tx,
    );

    join_tasks(bot, grpc_server, shutdown_task).await;
    deployment::mark_instance_stopped(&pool, &runtime).await?;

    Ok(())
}

fn install_crypto_provider() -> AppResult<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "Failed to install rustls crypto provider")?;
    Ok(())
}

async fn ensure_recording_dir() -> AppResult<()> {
    let recording_path = crate::events::voice_receiver::recording_file_path();
    if !recording_path.exists() {
        tokio::fs::create_dir_all(&recording_path).await?;
    }
    Ok(())
}

async fn connect_database() -> AppResult<Pool<Postgres>> {
    let db_url = crate::config::db_url()?;
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?)
}

async fn build_discord_client(
    pool: &Pool<Postgres>,
    runtime: Arc<crate::runtime::RuntimeState>,
    jam_cooldown: crate::cooldown::JamCooldown,
) -> AppResult<Client> {
    let discord_config = crate::config::discord_config()?;
    let songbird_config = Config::default()
        .decode_mode(DecodeMode::Decode(songbird::driver::DecodeConfig::default()));
    let intents = GatewayIntents::all();

    Ok(Client::builder(discord_config.token, intents)
        .event_handler(Handler {
            database: pool.clone(),
            jam_cooldown,
            runtime,
            afk_channels: Arc::new(RwLock::new(HashMap::new())),
        })
        .intents(intents)
        .register_songbird_from_config(songbird_config)
        .application_id(ApplicationId::new(discord_config.application_id))
        .await?)
}

async fn insert_typemap_state(client: &mut Client, runtime: Arc<crate::runtime::RuntimeState>) {
    let mut data = client.data.write().await;
    data.insert::<HasBossMusic>(HashMap::new());
    data.insert::<BotMetricsKey>(Arc::new(BotMetrics::default()));
    data.insert::<crate::runtime::RuntimeStateKey>(runtime);
}

async fn process_metrics(client: &Client) -> AppResult<Arc<BotMetrics>> {
    let data_read = client.data.read().await;
    data_read
        .get::<BotMetricsKey>()
        .cloned()
        .ok_or_else(|| "BotMetrics not inserted".into())
}

async fn join_tasks(
    bot: tokio::task::JoinHandle<()>,
    grpc_server: tokio::task::JoinHandle<()>,
    shutdown_task: tokio::task::JoinHandle<bool>,
) {
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
            tracing::warn!("Discord task aborted after shard shutdown timeout");
        } else {
            error!("Discord task join error: {}", err);
        }
    }
}

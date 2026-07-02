use std::{collections::HashMap, error::Error, sync::Arc};

use serenity::{all::ApplicationId, prelude::*};
use songbird::{Config, SerenityInit, driver::DecodeMode};
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tokio::task::{JoinError, JoinHandle};
use tracing::{error, info, warn};

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
    info!(
        instance_id = %runtime.config().instance_id,
        role = runtime.role().as_str(),
        drain_timeout_seconds = runtime.config().drain_timeout.as_secs(),
        "runtime configured"
    );

    let runtime_result = run_registered_instance(&pool, runtime.clone()).await;
    let cleanup_result = deployment::mark_instance_stopped(&pool, &runtime).await;

    match (runtime_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(cleanup_err)) => Err(cleanup_err.into()),
        (Err(runtime_err), Ok(())) => Err(runtime_err.into()),
        (Err(runtime_err), Err(cleanup_err)) => {
            error!(
                "instance cleanup failed after runtime failure: {}",
                cleanup_err
            );
            Err(runtime_err.into())
        }
    }
}

async fn run_registered_instance(
    pool: &Pool<Postgres>,
    runtime: Arc<crate::runtime::RuntimeState>,
) -> Result<(), RuntimeTaskError> {
    let jam_cooldown = crate::cooldown::JamCooldown::new();
    let mut client = build_discord_client(pool, runtime.clone(), jam_cooldown.clone())
        .await
        .map_err(RuntimeTaskError::Startup)?;
    insert_typemap_state(&mut client, runtime.clone()).await;

    let custom = Custom::new(
        client.cache.clone(),
        client.http.clone(),
        client.data.clone(),
        pool.to_owned(),
        jam_cooldown,
        runtime.clone(),
    );
    let process_metrics = process_metrics(&client)
        .await
        .map_err(RuntimeTaskError::Startup)?;
    BotMetrics::start_sysinfo_monitoring(process_metrics.clone());
    BotMetrics::register_otel_metrics(process_metrics, runtime.clone());

    let shard_manager = client.shard_manager.clone();
    let shutdown_data = client.data.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let discord = tokio::spawn(async move { client.start().await });
    let grpc = crate::grpc::spawn_server(custom.clone(), shutdown_rx.clone());
    let heartbeat =
        deployment::start_heartbeat(pool.to_owned(), runtime.clone(), shutdown_rx.clone());
    let reconciliation = crate::events::voice::spawn_reconciliation(custom, shutdown_rx.clone());
    let shutdown_monitor = crate::shutdown::spawn_shutdown_monitor(
        runtime.clone(),
        pool.to_owned(),
        shutdown_data.clone(),
        shutdown_rx,
    );

    let mut tasks = RuntimeTasks {
        discord,
        grpc,
        heartbeat,
        reconciliation,
        shutdown_monitor,
    };
    let first_exit = tasks.wait_for_first_exit().await;
    let selected = first_exit.kind();
    let decision = classify_first_exit(first_exit);

    if matches!(decision, SupervisorDecision::Fatal(_)) {
        runtime.force_shutdown();
    }

    let _ = shutdown_tx.send(true);
    crate::shutdown::clear_voice_presence_metrics(&shutdown_data).await;

    tasks.stop_background_tasks(selected).await;

    if tokio::time::timeout(
        crate::shutdown::SHARD_SHUTDOWN_TIMEOUT,
        shard_manager.shutdown_all(),
    )
    .await
    .is_err()
    {
        warn!(
            timeout_seconds = crate::shutdown::SHARD_SHUTDOWN_TIMEOUT.as_secs(),
            "shard shutdown timed out; aborting Discord client task"
        );
        tasks.discord.abort();
    }
    tasks.stop_discord(selected).await;

    match decision {
        SupervisorDecision::Graceful(trigger) => {
            info!(?trigger, "runtime shutdown completed");
            Ok(())
        }
        SupervisorDecision::Fatal(err) => Err(err),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskKind {
    Discord,
    Grpc,
    Heartbeat,
    Reconciliation,
    ShutdownMonitor,
}

impl TaskKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "Discord",
            Self::Grpc => "gRPC",
            Self::Heartbeat => "heartbeat",
            Self::Reconciliation => "reconciliation",
            Self::ShutdownMonitor => "shutdown monitor",
        }
    }
}

struct RuntimeTasks {
    discord: JoinHandle<Result<(), serenity::Error>>,
    grpc: JoinHandle<Result<(), crate::grpc::GrpcServerError>>,
    heartbeat: JoinHandle<()>,
    reconciliation: JoinHandle<()>,
    shutdown_monitor: JoinHandle<
        Result<crate::shutdown::ShutdownMonitorExit, crate::shutdown::ShutdownMonitorError>,
    >,
}

impl RuntimeTasks {
    async fn wait_for_first_exit(&mut self) -> ObservedTaskExit {
        tokio::select! {
            result = &mut self.discord => ObservedTaskExit::Discord(result),
            result = &mut self.grpc => ObservedTaskExit::Grpc(result),
            result = &mut self.heartbeat => ObservedTaskExit::Heartbeat(result),
            result = &mut self.reconciliation => ObservedTaskExit::Reconciliation(result),
            result = &mut self.shutdown_monitor => ObservedTaskExit::ShutdownMonitor(result),
        }
    }

    async fn stop_background_tasks(&mut self, selected: TaskKind) {
        stop_task(&mut self.grpc, selected == TaskKind::Grpc, TaskKind::Grpc).await;
        stop_task(
            &mut self.heartbeat,
            selected == TaskKind::Heartbeat,
            TaskKind::Heartbeat,
        )
        .await;
        stop_task(
            &mut self.reconciliation,
            selected == TaskKind::Reconciliation,
            TaskKind::Reconciliation,
        )
        .await;
        stop_task(
            &mut self.shutdown_monitor,
            selected == TaskKind::ShutdownMonitor,
            TaskKind::ShutdownMonitor,
        )
        .await;
    }

    async fn stop_discord(&mut self, selected: TaskKind) {
        stop_task(
            &mut self.discord,
            selected == TaskKind::Discord,
            TaskKind::Discord,
        )
        .await;
    }
}

const TASK_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn stop_task<T>(handle: &mut JoinHandle<T>, already_observed: bool, kind: TaskKind) {
    if already_observed {
        return;
    }

    match tokio::time::timeout(TASK_STOP_TIMEOUT, &mut *handle).await {
        Ok(Ok(_)) => {}
        Ok(Err(err)) if err.is_cancelled() => {}
        Ok(Err(err)) => warn!(task = kind.as_str(), "task failed during shutdown: {}", err),
        Err(_) => {
            warn!(task = kind.as_str(), "task did not stop; aborting");
            handle.abort();
            let _ = handle.await;
        }
    }
}

enum ObservedTaskExit {
    Discord(Result<Result<(), serenity::Error>, JoinError>),
    Grpc(Result<Result<(), crate::grpc::GrpcServerError>, JoinError>),
    Heartbeat(Result<(), JoinError>),
    Reconciliation(Result<(), JoinError>),
    ShutdownMonitor(
        Result<
            Result<crate::shutdown::ShutdownMonitorExit, crate::shutdown::ShutdownMonitorError>,
            JoinError,
        >,
    ),
}

impl ObservedTaskExit {
    fn kind(&self) -> TaskKind {
        match self {
            Self::Discord(_) => TaskKind::Discord,
            Self::Grpc(_) => TaskKind::Grpc,
            Self::Heartbeat(_) => TaskKind::Heartbeat,
            Self::Reconciliation(_) => TaskKind::Reconciliation,
            Self::ShutdownMonitor(_) => TaskKind::ShutdownMonitor,
        }
    }
}

enum SupervisorDecision {
    Graceful(crate::shutdown::GracefulShutdownTrigger),
    Fatal(RuntimeTaskError),
}

#[derive(Debug, thiserror::Error)]
enum RuntimeTaskError {
    #[error("runtime startup failed")]
    Startup(#[source] Box<dyn Error + Send + Sync>),
    #[error("Discord client failed")]
    Discord(#[source] serenity::Error),
    #[error("gRPC server failed")]
    Grpc(#[source] crate::grpc::GrpcServerError),
    #[error("shutdown monitor failed")]
    ShutdownMonitor(#[source] crate::shutdown::ShutdownMonitorError),
    #[error("{task} task exited unexpectedly")]
    UnexpectedExit { task: &'static str },
    #[error("{task} task join failed")]
    Join {
        task: &'static str,
        #[source]
        source: JoinError,
    },
}

fn classify_first_exit(exit: ObservedTaskExit) -> SupervisorDecision {
    match exit {
        ObservedTaskExit::Discord(Ok(Err(err))) => {
            SupervisorDecision::Fatal(RuntimeTaskError::Discord(err))
        }
        ObservedTaskExit::Grpc(Ok(Err(err))) => {
            SupervisorDecision::Fatal(RuntimeTaskError::Grpc(err))
        }
        ObservedTaskExit::ShutdownMonitor(Ok(Err(err))) => {
            SupervisorDecision::Fatal(RuntimeTaskError::ShutdownMonitor(err))
        }
        ObservedTaskExit::ShutdownMonitor(Ok(Ok(
            crate::shutdown::ShutdownMonitorExit::Triggered(trigger),
        ))) => SupervisorDecision::Graceful(trigger),
        ObservedTaskExit::Discord(Err(err)) => join_failure(TaskKind::Discord, err),
        ObservedTaskExit::Grpc(Err(err)) => join_failure(TaskKind::Grpc, err),
        ObservedTaskExit::Heartbeat(Err(err)) => join_failure(TaskKind::Heartbeat, err),
        ObservedTaskExit::Reconciliation(Err(err)) => join_failure(TaskKind::Reconciliation, err),
        ObservedTaskExit::ShutdownMonitor(Err(err)) => join_failure(TaskKind::ShutdownMonitor, err),
        ObservedTaskExit::Discord(Ok(Ok(()))) => unexpected_exit(TaskKind::Discord),
        ObservedTaskExit::Grpc(Ok(Ok(()))) => unexpected_exit(TaskKind::Grpc),
        ObservedTaskExit::Heartbeat(Ok(())) => unexpected_exit(TaskKind::Heartbeat),
        ObservedTaskExit::Reconciliation(Ok(())) => unexpected_exit(TaskKind::Reconciliation),
        ObservedTaskExit::ShutdownMonitor(Ok(Ok(
            crate::shutdown::ShutdownMonitorExit::Cancelled,
        ))) => unexpected_exit(TaskKind::ShutdownMonitor),
    }
}

fn join_failure(kind: TaskKind, source: JoinError) -> SupervisorDecision {
    SupervisorDecision::Fatal(RuntimeTaskError::Join {
        task: kind.as_str(),
        source,
    })
}

fn unexpected_exit(kind: TaskKind) -> SupervisorDecision {
    SupervisorDecision::Fatal(RuntimeTaskError::UnexpectedExit {
        task: kind.as_str(),
    })
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

#[cfg(test)]
mod tests {
    use super::{
        ObservedTaskExit, RuntimeTaskError, SupervisorDecision, TaskKind, classify_first_exit,
        stop_task,
    };

    #[test]
    fn graceful_drain_is_successful() {
        let decision = classify_first_exit(ObservedTaskExit::ShutdownMonitor(Ok(Ok(
            crate::shutdown::ShutdownMonitorExit::Triggered(
                crate::shutdown::GracefulShutdownTrigger::DrainComplete,
            ),
        ))));
        assert!(matches!(
            decision,
            SupervisorDecision::Graceful(crate::shutdown::GracefulShutdownTrigger::DrainComplete)
        ));
    }

    #[test]
    fn discord_failure_is_fatal() {
        let decision = classify_first_exit(ObservedTaskExit::Discord(Ok(Err(
            serenity::Error::Other("test failure"),
        ))));
        assert!(matches!(
            decision,
            SupervisorDecision::Fatal(RuntimeTaskError::Discord(_))
        ));
    }

    #[test]
    fn grpc_failure_is_fatal() {
        let parse_error = "invalid".parse::<std::net::SocketAddr>().unwrap_err();
        let decision = classify_first_exit(ObservedTaskExit::Grpc(Ok(Err(
            crate::grpc::GrpcServerError::InvalidAddress(parse_error),
        ))));
        assert!(matches!(
            decision,
            SupervisorDecision::Fatal(RuntimeTaskError::Grpc(_))
        ));
    }

    #[tokio::test]
    async fn heartbeat_panic_is_fatal() {
        let join_error = tokio::spawn(async {
            panic!("heartbeat panic");
        })
        .await
        .unwrap_err();
        let decision = classify_first_exit(ObservedTaskExit::Heartbeat(Err(join_error)));
        assert!(matches!(
            decision,
            SupervisorDecision::Fatal(RuntimeTaskError::Join {
                task: "heartbeat",
                ..
            })
        ));
    }

    #[test]
    fn unexpected_clean_exit_is_fatal() {
        let decision = classify_first_exit(ObservedTaskExit::Discord(Ok(Ok(()))));
        assert!(matches!(
            decision,
            SupervisorDecision::Fatal(RuntimeTaskError::UnexpectedExit { task: "Discord" })
        ));
    }

    #[tokio::test]
    async fn task_exit_after_shutdown_race_is_expected() {
        let mut handle = tokio::spawn(async {});
        stop_task(&mut handle, false, TaskKind::Grpc).await;
    }
}

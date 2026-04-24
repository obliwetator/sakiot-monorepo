use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tracing::{error, info};

use crate::config::GRPC_ADDRESS;
use crate::errors::AppError;

pub mod hello_world {
    #![allow(non_snake_case)]
    tonic::include_proto!("helloworld");
}

use hello_world::dashboard_client::DashboardClient;
use hello_world::ClientMessage;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);

struct DashboardWebSocket {
    topic_tx: watch::Sender<String>,
    last_heartbeat: Instant,
}

impl Actor for DashboardWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Dashboard WebSocket connected");
        self.start_heartbeat(ctx);

        let addr = ctx.address();
        let topic_rx = self.topic_tx.subscribe();

        tokio::spawn(async move {
            loop {
                info!("Attempting to connect to Dashboard gRPC service...");
                let mut client = match DashboardClient::connect(GRPC_ADDRESS.as_str()).await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(
                            "Failed to connect to Dashboard gRPC service: {}. Retrying in 3s...",
                            e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        continue;
                    }
                };

                info!("Successfully connected to Dashboard gRPC service");

                let (tx, rx) = mpsc::channel(100);

                // Send the initial topic immediately upon connection
                let initial_topic = topic_rx.borrow().clone();
                if !initial_topic.is_empty() {
                    let _ = tx
                        .send(ClientMessage {
                            action: "subscribe".to_string(),
                            topic: initial_topic,
                        })
                        .await;
                }

                // Spawn a task to forward watch changes to the mpsc stream
                let tx_clone = tx.clone();
                let mut task_rx = topic_rx.clone();
                tokio::spawn(async move {
                    while task_rx.changed().await.is_ok() {
                        let t = task_rx.borrow().clone();
                        let action = if t.is_empty() { "unsubscribe" } else { "subscribe" };
                        if tx_clone
                            .send(ClientMessage {
                                action: action.to_string(),
                                topic: t,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });

                let req_stream = ReceiverStream::new(rx);
                let request = tonic::Request::new(req_stream);

                let mut stream = match client.dashboard_stream(request).await {
                    Ok(response) => response.into_inner(),
                    Err(e) => {
                        error!("Failed to start dashboard stream: {}. Retrying in 3s...", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        continue;
                    }
                };

                while let Some(event_result) = stream.next().await {
                    match event_result {
                        Ok(event) => {
                            let json = serde_json::json!({
                                "event_type": event.event_type,
                                "payload": event.json_payload,
                            });

                            if addr.try_send(MetricsMessage(json.to_string())).is_err() {
                                return; // Actor closed, terminate entirely
                            }
                        }
                        Err(e) => {
                            error!("gRPC stream error: {}. Disconnected.", e);
                            break; // Break inner loop to trigger reconnect
                        }
                    }
                }

                info!("gRPC stream ended. Reconnecting in 3s...");
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }
        });
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        info!("Dashboard WebSocket disconnected");
    }
}

impl DashboardWebSocket {
    fn start_heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.last_heartbeat) > CLIENT_TIMEOUT {
                info!("Dashboard WebSocket client timed out, stopping");
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }
}

struct MetricsMessage(String);

impl actix::Message for MetricsMessage {
    type Result = ();
}

impl actix::Handler<MetricsMessage> for DashboardWebSocket {
    type Result = ();

    fn handle(&mut self, msg: MetricsMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for DashboardWebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.last_heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.last_heartbeat = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let (Some(action), Some(topic)) =
                        (json["action"].as_str(), json["topic"].as_str())
                    {
                        if action == "subscribe" {
                            let _ = self.topic_tx.send(topic.to_string());
                        } else if action == "unsubscribe" {
                            let _ = self.topic_tx.send(String::new());
                        }
                    }
                }
            }
            Ok(ws::Message::Binary(_)) => (),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

#[get("/dashboard/stream")]
pub async fn dashboard_stream(
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, AppError> {
    let (topic_tx, _) = watch::channel(String::new());
    ws::start(
        DashboardWebSocket {
            topic_tx,
            last_heartbeat: Instant::now(),
        },
        &req,
        stream,
    )
    .map_err(|e| {
        tracing::error!(?e, "websocket start failed");
        AppError::InternalError
    })
}

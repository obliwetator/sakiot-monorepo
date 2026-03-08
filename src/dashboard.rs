use actix::{Actor, StreamHandler, AsyncContext, ActorContext};
use actix_web::{get, web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use tracing::{error, info};
use tokio_stream::StreamExt;

pub mod hello_world {
    #![allow(non_snake_case)]
    tonic::include_proto!("helloworld");
}

use hello_world::dashboard_client::DashboardClient;
use hello_world::Empty;

use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use hello_world::ClientMessage;

struct DashboardWebSocket {
    topic_tx: watch::Sender<String>,
}

impl Actor for DashboardWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Dashboard WebSocket connected");

        let addr = ctx.address();
        let topic_rx = self.topic_tx.subscribe();
        
        tokio::spawn(async move {
            loop {
                info!("Attempting to connect to Dashboard gRPC service...");
                let mut client = match DashboardClient::connect("http://[::1]:50052").await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to connect to Dashboard gRPC service: {}. Retrying in 3s...", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        continue;
                    }
                };

                info!("Successfully connected to Dashboard gRPC service");

                let (tx, rx) = mpsc::channel(100);
                
                // Send the initial topic immediately upon connection
                let initial_topic = topic_rx.borrow().clone();
                if !initial_topic.is_empty() {
                    let _ = tx.send(ClientMessage { action: "subscribe".to_string(), topic: initial_topic }).await;
                }

                // Spawn a task to forward watch changes to the mpsc stream
                let tx_clone = tx.clone();
                let mut task_rx = topic_rx.clone();
                tokio::spawn(async move {
                    while task_rx.changed().await.is_ok() {
                        let t = task_rx.borrow().clone();
                        let action = if t.is_empty() { "unsubscribe" } else { "subscribe" };
                        if tx_clone.send(ClientMessage { action: action.to_string(), topic: t }).await.is_err() {
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
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Text(text)) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let (Some(action), Some(topic)) = (json["action"].as_str(), json["topic"].as_str()) {
                        if action == "subscribe" {
                            let _ = self.topic_tx.send(topic.to_string());
                        } else if action == "unsubscribe" {
                            let _ = self.topic_tx.send(String::new());
                        }
                    }
                }
            },
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
pub async fn dashboard_stream(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, actix_web::Error> {
    let (topic_tx, _) = watch::channel(String::new());
    ws::start(DashboardWebSocket { topic_tx }, &req, stream)
}

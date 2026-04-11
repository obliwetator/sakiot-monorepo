use std::time::{Duration, Instant};
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web::{get, web, Error, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use tracing::{error, info};

/// Define HTTP actor
#[derive(Debug)]
struct MyWs {
    hb: Instant,
    authorized: bool,
}

impl Actor for MyWs {
    type Context = ws::WebsocketContext<Self>;
}

impl MyWs {
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > HEARTBEAT_INTERVAL + CLIENT_TIMEOUT {
                error!("Websocket Client heartbeat failed, disconnecting!");

                ctx.stop();
                return;
            }

            ctx.ping(b"");
        });
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWs {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        info!("WS Msg {:#?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg)
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            _ => (),
        }
    }

    fn started(&mut self, ctx: &mut Self::Context) {
        if !self.authorized {
            tracing::info!("Websocket not authorized. Closing connection with custom code 4001");
            ctx.close(Some(ws::CloseReason {
                code: ws::CloseCode::Other(4001),
                description: Some("Unauthorized".to_string()),
            }));
            ctx.stop();
            return;
        }
        self.hb(ctx);
    }
}

#[get("/ws/")]
pub async fn web_socket(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    use crate::auth::{get_access_and_refresh_tokens, Access, Token};
    use crate::AccessKeys;
    use actix_web::web::Data;

    let headers = req.headers();
    let mut is_authorized = false;

    if let Some(cookie) = headers.get("cookie") {
        if let Some(keys) = req.app_data::<Data<AccessKeys>>() {
            if let Ok((access_token, _)) = get_access_and_refresh_tokens(cookie) {
                if Token::<Access>::decode(access_token, keys).is_ok() {
                    is_authorized = true;
                }
            }
        }
    }

    if !is_authorized {
        tracing::warn!(
            "Unauthorized access attempt to websocket {}: missing or invalid token",
            req.path()
        );
    }

    let resp = ws::WsResponseBuilder::new(
        MyWs {
            hb: Instant::now(),
            authorized: is_authorized,
        },
        &req,
        stream,
    )
    .start();
    // let resp = ws::start(MyWs {}, &req, stream);
    error!("{:?}", resp);
    resp
}

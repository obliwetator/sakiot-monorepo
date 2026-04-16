use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use sqlx::postgres::PgPoolOptions;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use web_server::audio::{
    download_audio, find_similar, get_audio, get_waveform_data, remove_silence,
};
use web_server::auth::{discord_login, get_token, logout, refresh_jwt};
use web_server::clips::{create_clip, delete, get_clip, get_clips, play_clip};
use web_server::dashboard;
use web_server::stamps::get_stamps;
use web_server::grpc::hello_world::greeter_server::GreeterServer;
use web_server::grpc::MyGreeter;
use web_server::secrets::{ACCESS_SECRET, REFRESH_SECRET};
use web_server::user::{get_current_user, get_current_user_guilds};
use web_server::websocket::web_socket;
use web_server::{
    get_current_month_permission, AccessKeys, AuthMiddleware, HashMapContainer,
    WaveformProgressContainer,
};

use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::transport::Server;

#[get("/current/{guild_id}")]
async fn perm_calc(
    _path: web::Path<String>,
    _token: Option<web::ReqData<web_server::Token<web_server::Access>>>,
    _pool: web::Data<sqlx::Pool<sqlx::Postgres>>,
) -> impl Responder {
    HttpResponse::Ok()
}

async fn not_found() -> impl Responder {
    HttpResponse::NotFound().json("not found")
}

#[get("/download_the_clip")]
async fn download_the_clip() -> Result<actix_files::NamedFile, actix_web::Error> {
    Ok(actix_files::NamedFile::open("./clips.tar.gz")?)
}

#[actix_web::main]
async fn main() {
    env_logger::init();

    let hashmap = web::Data::new(HashMapContainer(RwLock::new(HashMap::new())));
    let waveform_progress = web::Data::new(WaveformProgressContainer(RwLock::new(HashMap::new())));

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgres://postgres:okcpli4t94@localhost/sakiot_rouvas")
        .await
        .expect("cannot connect to database");

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .pretty()
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let server: actix_web::dev::Server = HttpServer::new(move || {
        let logger = Logger::default();
        let keys = AccessKeys {
            access_encode: jsonwebtoken::EncodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_encode: jsonwebtoken::EncodingKey::from_secret(REFRESH_SECRET.as_bytes()),
            access_decode: jsonwebtoken::DecodingKey::from_secret(ACCESS_SECRET.as_bytes()),
            refresh_decode: jsonwebtoken::DecodingKey::from_secret(REFRESH_SECRET.as_bytes()),
        };

        let api_scope = web::scope("/api")
            .wrap(AuthMiddleware)
            .service(discord_login)
            .service(refresh_jwt)
            .service(logout)
            .service(get_current_user)
            .service(get_current_user_guilds)
            .service(get_token)
            .service(find_similar)
            .service(get_current_month_permission)
            .service(perm_calc)
            .service(remove_silence)
            .service(delete)
            .service(dashboard::dashboard_stream)
            .service(get_clips)
            .service(get_clip)
            .service(get_stamps)
            .service(play_clip)
            .service(create_clip)
            .service(get_audio)
            .service(get_waveform_data)
            .service(download_audio);
        App::new()
            .wrap(logger)
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(reqwest::Client::new()))
            .app_data(hashmap.clone())
            .app_data(waveform_progress.clone())
            .app_data(web::Data::new(keys))
            .service(web_socket)
            .service(api_scope)
            .service(download_the_clip)
            .default_service(web::route().to(not_found))
            .wrap(Cors::permissive())
    })
    .bind(("127.0.0.1", 8900))
    .expect("bind 127.0.0.1:8900 failed")
    .run();

    let _tonic = tokio::spawn(async move {
        let addr = "[::1]:50051".parse().expect("valid gRPC listen addr");
        let greeter = MyGreeter::default();

        info!("GreeterServer listening on {}", addr);

        Server::builder()
            .add_service(GreeterServer::new(greeter))
            .serve(addr)
            .await
    });

    let _c = tokio::spawn(async move { server.await });
    let _res = tokio::join!(_c, _tonic);
}

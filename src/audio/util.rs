use std::path::Path;

use actix_web::HttpRequest;
use tracing::error;

pub fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let guild_id = &path.0;
    let channel_id = &path.1;
    let year = &path.2;
    let month = &path.3;

    format!("{}{}/{}/{}/{}", base_path, guild_id, channel_id, year, month)
}

pub fn file_exists(path: &str) -> bool {
    Path::new(path).try_exists().unwrap_or(false)
}

pub fn handle_idempotency_key(req: &HttpRequest) -> Result<String, ()> {
    let header = match req.headers().get("Idempotency-Key") {
        Some(ok) => ok,
        None => {
            error!("Idempotency key is missing");
            return Err(());
        }
    };

    match header.to_str() {
        Ok(ok) => Ok(ok.to_owned()),
        Err(_) => {
            error!("No value in Idempotency header");
            Err(())
        }
    }
}

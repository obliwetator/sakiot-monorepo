use actix_web::HttpRequest;
use tracing::error;

pub fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let key = sakiot_paths::RecordingKey::new(path.0, path.1, path.2, path.3 as u32, &path.4);
    key.recording_dir(base_path).to_string_lossy().into_owned()
}

pub async fn file_exists(path: &str) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

pub fn handle_idempotency_key(req: &HttpRequest) -> Result<String, crate::errors::AppError> {
    let header = match req.headers().get("Idempotency-Key") {
        Some(ok) => ok,
        None => {
            error!("Idempotency key is missing");
            return Err(crate::errors::AppError::BadRequest("Idempotency key is missing".into()));
        }
    };

    match header.to_str() {
        Ok(ok) => Ok(ok.to_owned()),
        Err(_) => {
            error!("No value in Idempotency header");
            Err(crate::errors::AppError::BadRequest("No value in Idempotency header".into()))
        }
    }
}

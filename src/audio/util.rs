use actix_web::HttpRequest;
use tracing::error;

pub fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let key = sakiot_paths::RecordingKey::new(path.0, path.1, path.2, path.3 as u32, &path.4);
    key.recording_dir(base_path).to_string_lossy().into_owned()
}

/// Padded + unpadded dir variants, for legacy-layout compatibility
/// (pre-unification files live under `YYYY/M`, new writes under `YYYY/MM`).
pub fn get_file_path_root_candidates(
    base_path: &str,
    path: &(i64, i64, i32, i32, String),
) -> [String; 2] {
    let padded = get_file_path_root(base_path, path);
    let unpadded = format!("{}{}/{}/{}/{}", base_path, path.0, path.1, path.2, path.3);
    [padded, unpadded]
}

pub async fn resolve_existing_dir(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let cands = get_file_path_root_candidates(base_path, path);
    for c in &cands {
        if tokio::fs::try_exists(c).await.unwrap_or(false) {
            return c.clone();
        }
    }
    cands.into_iter().next().unwrap_or_default()
}

pub async fn file_exists(path: &str) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
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

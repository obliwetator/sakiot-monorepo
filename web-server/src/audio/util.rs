use actix_web::HttpRequest;
use tracing::error;

/// Whether a request-supplied file, stem, or segment name is safe to
/// interpolate into a filesystem path or a process argument: non-empty, no
/// parent-directory or path-separator sequences, no quotes, and no control
/// characters. Every audio handler that builds a path or command line from a
/// URL segment must gate it through this predicate.
pub fn is_valid_file_segment(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\'')
        && !s.contains('"')
        && !s.chars().any(char::is_control)
}

pub fn get_file_path_root(base_path: &str, path: &(i64, i64, i32, i32, String)) -> String {
    let key = sakiot_paths::RecordingKey::new(path.0, path.1, path.2, path.3 as u32, &path.4);
    key.recording_dir(base_path).to_string_lossy().into_owned()
}

pub async fn file_exists(path: &str) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

/// Whether `derived` needs regenerating from `source`: true when `derived` is
/// missing or older than `source`. If `source` is missing we can't regenerate,
/// so the existing `derived` is kept (returns false). Used to invalidate cached
/// artifacts produced from a recording that has since grown (e.g. mid-live).
pub async fn is_stale(source: &str, derived: &str) -> bool {
    let Ok(derived_meta) = tokio::fs::metadata(derived).await else {
        return true;
    };
    let Ok(source_meta) = tokio::fs::metadata(source).await else {
        return false;
    };
    match (source_meta.modified(), derived_meta.modified()) {
        (Ok(source_mtime), Ok(derived_mtime)) => source_mtime > derived_mtime,
        _ => false,
    }
}

pub fn handle_idempotency_key(req: &HttpRequest) -> Result<String, crate::errors::AppError> {
    let header = match req.headers().get("Idempotency-Key") {
        Some(ok) => ok,
        None => {
            error!("Idempotency key is missing");
            return Err(crate::errors::AppError::BadRequest(
                "Idempotency key is missing".into(),
            ));
        }
    };

    match header.to_str() {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Err(crate::errors::AppError::BadRequest(
                    "Idempotency key is empty".into(),
                ));
            }
            if value.len() > 255 {
                return Err(crate::errors::AppError::BadRequest(
                    "Idempotency key exceeds 255 bytes".into(),
                ));
            }
            Ok(value.to_owned())
        }
        Err(_) => {
            error!("No value in Idempotency header");
            Err(crate::errors::AppError::BadRequest(
                "No value in Idempotency header".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_idempotency_key, is_valid_file_segment};
    use actix_web::test::TestRequest;

    #[test]
    fn file_segment_accepts_generated_names() {
        assert!(is_valid_file_segment("1786473460682-183931044829986817"));
        assert!(is_valid_file_segment(
            "1786473460682-183931044829986817.ogg"
        ));
        assert!(is_valid_file_segment("seg_00001.m4s"));
    }

    #[test]
    fn file_segment_rejects_traversal_and_separators() {
        assert!(!is_valid_file_segment(""));
        assert!(!is_valid_file_segment(".."));
        assert!(!is_valid_file_segment("../secret"));
        assert!(!is_valid_file_segment("a..b"));
        assert!(!is_valid_file_segment("dir/file"));
        assert!(!is_valid_file_segment("dir\\file"));
    }

    #[test]
    fn file_segment_rejects_shell_metacharacters() {
        assert!(!is_valid_file_segment("bad'name"));
        assert!(!is_valid_file_segment("bad\"name"));
        assert!(!is_valid_file_segment("bad\nname"));
        assert!(!is_valid_file_segment("bad\tname"));
    }

    #[test]
    fn idempotency_key_is_trimmed() -> Result<(), Box<dyn std::error::Error>> {
        let req = TestRequest::default()
            .insert_header(("Idempotency-Key", " request-123 "))
            .to_http_request();

        assert_eq!(handle_idempotency_key(&req)?, "request-123");
        Ok(())
    }

    #[test]
    fn idempotency_key_rejects_empty_values() {
        let req = TestRequest::default()
            .insert_header(("Idempotency-Key", "   "))
            .to_http_request();

        assert!(handle_idempotency_key(&req).is_err());
    }
}

use actix_web::{HttpResponse, error::ResponseError, http::StatusCode};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ApiError {
    pub code: u16,
    pub message: String,
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Clip not found")]
    ClipNotFound,
    #[error("Role not found in this guild")]
    RoleNotFound,
    #[error("File not found on disk")]
    FileNotFound,
    #[error("File could not be deleted from disk")]
    FileDeleteFailed,
    #[error("Forbidden")]
    Forbidden,
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Internal Server Error")]
    InternalError,
    #[error("Database Error: {0}")]
    DbError(#[from] sqlx::Error),
    #[error("Request Error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("IO Error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Parse Error: {0}")]
    ParseError(#[from] std::num::ParseIntError),
    #[error("JWT Error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
    #[error("HTTP Error: {0}")]
    HttpError(#[from] actix_web::error::HttpError),
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Invalid path param: {0}")]
    InvalidParam(String),
    #[error("FFmpeg failed: {0}")]
    FfmpegError(String),
    /// The request was valid but a local media tool (ffprobe/ffmpeg) could not
    /// produce a trustworthy answer about the source. Distinct from
    /// `FfmpegError` (a command that ran and failed) so callers can tell a
    /// media-inspection failure apart from a server fault.
    #[error("Bad Gateway: {0}")]
    BadGateway(String),
    #[error("Upstream gRPC error: {0}")]
    GrpcError(String),
    #[error("Service Unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Requested range is not satisfiable")]
    RangeNotSatisfiable { total: u64 },
    #[error("Invalid or expired token")]
    InvalidToken,
}

/// The message sent to the client. Server faults must not echo internal detail
/// (SQL, paths, secrets); client errors keep their explanatory text.
fn client_message(status: StatusCode, error: &AppError) -> String {
    if status.is_server_error() {
        status.canonical_reason().unwrap_or("Error").to_string()
    } else {
        error.to_string()
    }
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::ClipNotFound => StatusCode::NOT_FOUND,
            AppError::RoleNotFound => StatusCode::NOT_FOUND,
            AppError::FileNotFound => StatusCode::NOT_FOUND,
            AppError::FileDeleteFailed => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::InvalidToken => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            AppError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::BadGateway(_) => StatusCode::BAD_GATEWAY,
            AppError::RangeNotSatisfiable { .. } => StatusCode::RANGE_NOT_SATISFIABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let status_code = self.status_code();
        if status_code.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = ?self, "request rejected");
        }
        let message = client_message(status_code, self);
        let error_response = ApiError {
            code: status_code.as_u16(),
            message,
        };
        let mut response = HttpResponse::build(status_code);
        if let AppError::RangeNotSatisfiable { total } = self {
            response.insert_header((
                actix_web::http::header::CONTENT_RANGE,
                format!("bytes */{total}"),
            ));
        }
        response.json(error_response)
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;
    use actix_web::{HttpResponse, error::ResponseError, http::StatusCode};

    #[test]
    fn media_inspection_failures_are_bad_gateways_not_server_faults() {
        // A recording ffprobe cannot measure is an upstream media problem: the
        // client's request may be perfectly valid, so it must not look like a
        // bug in this server.
        assert_eq!(
            AppError::BadGateway("ffprobe returned no audio duration".into()).status_code(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            AppError::BadRequest("Clip duration must be between 1 and 20 seconds".into())
                .status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::InternalError.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn server_faults_do_not_leak_internal_detail_to_clients() {
        let response = AppError::InternalError.error_response();
        assert_eq!(
            response.status(),
            HttpResponse::InternalServerError().finish().status()
        );

        // The body must carry the canonical reason, never the internal error
        // text (SQL, paths, or anything else the operator needs to see).
        let leaky = AppError::DbError(sqlx::Error::RowNotFound);
        let message = super::client_message(StatusCode::INTERNAL_SERVER_ERROR, &leaky);
        assert_eq!(message, "Internal Server Error");
        assert!(!message.contains("RowNotFound"));
        assert!(!message.contains("Database"));
        // Client errors keep their explanatory text.
        assert_eq!(
            super::client_message(StatusCode::BAD_REQUEST, &AppError::BadRequest("bad".into())),
            "Bad Request: bad"
        );
    }
}

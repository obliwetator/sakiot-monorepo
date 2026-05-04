use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Clip not found")]
    ClipNotFound,
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
    #[error("Invalid path param: {0}")]
    InvalidParam(String),
    #[error("FFmpeg failed: {0}")]
    FfmpegError(String),
    #[error("Upstream gRPC error: {0}")]
    GrpcError(String),
    #[error("Service Unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Invalid or expired token")]
    InvalidToken,
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::ClipNotFound => StatusCode::NOT_FOUND,
            AppError::FileNotFound => StatusCode::NOT_FOUND,
            AppError::FileDeleteFailed => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::InvalidToken => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            AppError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
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
        let message = if status_code.is_server_error() {
            status_code.canonical_reason().unwrap_or("Error").to_string()
        } else {
            self.to_string()
        };
        let error_response = serde_json::json!({
            "code": status_code.as_u16(),
            "message": message,
        });
        HttpResponse::build(status_code).json(error_response)
    }
}


use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
use serde::Serialize;
use serde_repr::Serialize_repr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not Found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
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
    #[error("Bad Request: {0}")]
    BadRequest(String),
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let status_code = self.status_code();
        let error_response = serde_json::json!({
            "code": status_code.as_u16(),
            "message": self.to_string(),
        });
        HttpResponse::build(status_code).json(error_response)
    }
}

#[derive(Serialize)]
pub struct ApiResponse {
    code: StatusCodes,
    message: String,
}

macro_rules! build_vararg_fn {
    ($name:tt, $status:expr, $msg:expr) => {
        #[allow(non_snake_case)]
        pub fn $name() -> Self {
            Self {
                code: $status,
                message: $msg.to_string(),
            }
        }
    };
}

impl ApiResponse {
    build_vararg_fn!(OK, StatusCodes::OK, "success");
    build_vararg_fn!(
        FILE_NOT_FOUND,
        StatusCodes::NotFound,
        "This file cannot be found"
    );
    build_vararg_fn!(
        FILE_ALREADY_DELETED,
        StatusCodes::NotFound,
        "This file has already been deleted"
    );
}

#[derive(Serialize_repr)]
#[repr(u32)]
#[derive(Eq, Hash, PartialEq)]
enum StatusCodes {
    OK,
    NotFound,
}

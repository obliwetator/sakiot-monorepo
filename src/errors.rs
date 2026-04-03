// use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
// use serde::Serialize;
// use thiserror::Error;

// #[derive(Error, Debug)]
// pub enum CustomError {
//     #[error("Requested file was not found")]
//     NotFound(),
//     #[error("You are forbidden to access requested file.")]
//     Forbidden(),
//     #[error("Unknown Internal Error")]
//     Unknown(),
// }
// impl CustomError {
//     pub fn name(&self) -> Cus {
//         match self {
//             Self::NotFound() => Cus {
//                 name: "NotFound".to_string(),
//             },
//             Self::Forbidden() => Cus {
//                 name: "Forbidden".to_string(),
//             },
//             Self::Unknown() => Cus {
//                 name: "Unknown".to_string(),
//             },
//         }
//     }
// }

// pub struct Cus {
//     name: String,
// }
// impl ResponseError for CustomError {
//     fn status_code(&self) -> StatusCode {
//         match *self {
//             Self::NotFound() => StatusCode::NOT_FOUND,
//             Self::Forbidden() => StatusCode::FORBIDDEN,
//             Self::Unknown() => StatusCode::INTERNAL_SERVER_ERROR,
//         }
//     }

//     fn error_response(&self) -> HttpResponse {
//         let status_code = self.status_code();
//         let error_response = ErrorResponse {
//             code: status_code.as_u16(),
//             message: self.to_string(),
//             error: self.name().name,
//         };
//         HttpResponse::build(status_code).json(error_response)
//     }
// }

// pub fn map_io_error(e: std::io::Error, v: u32) -> CustomError {
//     match e.kind() {
//         std::io::ErrorKind::NotFound => CustomError::NotFound(),
//         std::io::ErrorKind::PermissionDenied => CustomError::Forbidden(),
//         _ => CustomError::Unknown(),
//     }
// }

use serde::Serialize;
use serde_repr::Serialize_repr;

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

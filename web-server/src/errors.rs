//! The public error contract.
//!
//! Every failure a client can observe — an HTTP error body or a background
//! job's status — carries a stable machine-readable [`ErrorKind`] and a message
//! that is safe to display. Internal detail (SQL, filesystem paths, subprocess
//! output, upstream response bodies) is logged and never serialized.

use std::borrow::Cow;

use actix_web::{HttpResponse, error::ResponseError, http::StatusCode, web};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ApiError {
    /// The HTTP status code, repeated for clients that only keep the body.
    pub code: u16,
    pub kind: ErrorKind,
    /// A safe, human-readable explanation.
    pub message: String,
}

/// Declares [`ErrorKind`] from one table so the wire name, the parser used for
/// persisted job kinds, and the default public message cannot drift apart.
macro_rules! error_kinds {
    ($($(#[doc = $doc:literal])* $variant:ident = $name:tt => $message:literal,)*) => {
        /// Stable machine-readable error classification. Values are part of the
        /// API and are persisted by background jobs: add new ones, never rename.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
        pub enum ErrorKind {
            $($(#[doc = $doc])* #[serde(rename = $name)] $variant,)*
        }

        impl ErrorKind {
            #[cfg(test)]
            pub(crate) const ALL: &[Self] = &[$(Self::$variant),*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// `None` for a kind this release does not know, such as one
            /// persisted by a newer release running alongside it.
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// The explanation shown when nothing more specific is known.
            pub fn default_message(self) -> &'static str {
                match self {
                    $(Self::$variant => $message,)*
                }
            }
        }
    };
}

error_kinds! {
    /// The request failed validation; the message says what to change.
    InvalidRequest = "invalid_request" => "The request was invalid.",
    Unauthorized = "unauthorized" => "Your session is missing or has expired. Please log in again.",
    /// The anti-forgery token did not match the session.
    CsrfRejected = "csrf_rejected" => "Your session's security token is out of date. Reload the page and try again.",
    Forbidden = "forbidden" => "You do not have permission to do that.",
    NotFound = "not_found" => "The requested item was not found.",
    ClipNotFound = "clip_not_found" => "The clip was not found. It may have been deleted.",
    RoleNotFound = "role_not_found" => "That role was not found in this server.",
    /// A recording or media file is missing, typically because it was deleted.
    MediaNotFound = "media_not_found" => "The recording or media file was not found. It may have been deleted.",
    Conflict = "conflict" => "The request conflicts with the current state. Refresh and try again.",
    RangeNotSatisfiable = "range_not_satisfiable" => "The requested byte range is not available.",
    /// The caller's own active media jobs are at the per-user limit.
    UserJobLimitReached = "user_job_limit_reached" => "You already have the maximum number of active media jobs. Wait for one to finish, then try again.",
    /// The shared export queue is at capacity for everyone.
    ExportQueueFull = "export_queue_full" => "The export queue is full. Try again after other exports finish.",
    ExecutionTimedOut = "execution_timed_out" => "Processing exceeded its time limit and was stopped.",
    MediaTemporarilyUnavailable = "media_temporarily_unavailable" => "The media archive is temporarily unavailable. Try again later.",
    /// Archived bytes disagree with the verified ledger; retrying cannot help.
    ArchiveIntegrityFailure = "archive_integrity_failure" => "Archived media failed an integrity check. An administrator needs to investigate before it can be used.",
    MediaInspectionFailed = "media_inspection_failed" => "The server could not read this media file.",
    MediaProcessingFailed = "media_processing_failed" => "The server could not process this media.",
    MediaToolsUnavailable = "media_tools_unavailable" => "Media processing is unavailable on the server. An administrator needs to check its FFmpeg installation.",
    DiscordRateLimited = "discord_rate_limited" => "Discord is rate limiting requests. Try again shortly.",
    DiscordTimeout = "discord_timeout" => "Discord did not respond in time. Try again.",
    DiscordUnavailable = "discord_unavailable" => "Discord is temporarily unavailable. Try again later.",
    BotUnavailable = "bot_unavailable" => "The recording bot could not be reached.",
    /// A background worker stopped (crashed, restarted, or lost its lease)
    /// before the job finished.
    WorkerInterrupted = "worker_interrupted" => "The worker processing this job stopped before it finished.",
    SourceChanged = "source_changed" => "A source clip changed after this export was submitted.",
    SourceAccessRevoked = "source_access_revoked" => "Access to a source clip was removed.",
    DestinationChanged = "destination_changed" => "The destination clip changed or was deleted during export. Reopen it before overwriting.",
    StorageBudgetExceeded = "storage_budget_exceeded" => "The export needed more temporary storage than allowed. Shorten the edit and try again.",
    /// Deletion is paused until guild media jobs drain; not a failure.
    WaitingForMediaWork = "waiting_for_media_work" => "Waiting for this server's in-progress media jobs to finish.",
    /// A stored identifier or file name failed a safety check before use.
    UnsafeMediaReference = "unsafe_media_reference" => "A stored media reference failed a safety check. An administrator needs to investigate.",
    InternalError = "internal_error" => "Something went wrong on the server.",
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Clip not found")]
    ClipNotFound,
    #[error("Role not found in this guild")]
    RoleNotFound,
    /// A recording or media file is missing.
    #[error("File not found on disk")]
    FileNotFound,
    /// Any other missing resource, such as a job record.
    #[error("Not found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("Unauthorized")]
    Unauthorized,
    #[error("CSRF token mismatch")]
    CsrfRejected,
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
    /// Validation failure. The message is sent to the client verbatim, so it
    /// must describe the request, never server internals.
    #[error("Bad Request: {0}")]
    BadRequest(String),
    /// Same public-message contract as [`AppError::BadRequest`].
    #[error("Conflict: {0}")]
    Conflict(String),
    /// Names the malformed parameter; shown to the client.
    #[error("Invalid path param: {0}")]
    InvalidParam(String),
    /// A media command ran and failed. The detail (stderr) is for logs only.
    #[error("FFmpeg failed: {0}")]
    FfmpegError(String),
    /// The request was valid but a local media tool (ffprobe/ffmpeg) could not
    /// produce a trustworthy answer about the source. Distinct from
    /// `FfmpegError` (a command that ran and failed) so callers can tell a
    /// media-inspection failure apart from a server fault.
    #[error("Media inspection failed: {0}")]
    MediaInspectionFailed(String),
    #[error("{0} executable is unavailable; install FFmpeg on the web server")]
    MediaToolUnavailable(&'static str),
    /// The bot's gRPC endpoint could not be reached.
    #[error("Bot unavailable: {0}")]
    BotUnavailable(String),
    /// The bot was reached but the call failed.
    #[error("Upstream gRPC error: {0}")]
    GrpcError(String),
    #[error("The user already has the maximum number of active media jobs")]
    UserJobLimitReached,
    #[error("The export queue is full")]
    ExportQueueFull,
    #[error("Job exceeded its execution deadline")]
    ExecutionTimedOut,
    /// An export's pinned source clip no longer matches its snapshot.
    #[error("A source clip changed after the export was submitted")]
    SourceChanged,
    /// The clip an export overwrites changed or was deleted meanwhile.
    #[error("The destination clip changed or was deleted during export")]
    DestinationChanged,
    #[error("Export exceeded the temporary storage budget")]
    StorageBudgetExceeded,
    /// A background attempt no longer owns its job; it must stop without
    /// writing status. Never expected to reach an HTTP client.
    #[error("Job lease lost")]
    JobLeaseLost,
    /// A background attempt stopped for a reason outside the job itself,
    /// such as a crashed child process or an unreachable database.
    #[error("Worker interrupted: {0}")]
    WorkerInterrupted(String),
    #[error("Media archive unavailable: {0}")]
    MediaArchiveUnavailable(String),
    #[error("Archived media failed integrity validation: {0}")]
    ArchiveIntegrityFailure(String),
    #[error("Discord is unavailable: {0}")]
    DiscordUnavailable(String),
    #[error("Discord is rate limiting requests")]
    DiscordRateLimited { retry_after: Option<u64> },
    #[error("Discord did not respond in time")]
    DiscordTimeout,
    #[error("Requested range is not satisfiable")]
    RangeNotSatisfiable { total: u64 },
    #[error("Invalid or expired token")]
    InvalidToken,
}

impl AppError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::ClipNotFound => ErrorKind::ClipNotFound,
            Self::RoleNotFound => ErrorKind::RoleNotFound,
            Self::FileNotFound => ErrorKind::MediaNotFound,
            Self::NotFound => ErrorKind::NotFound,
            Self::Forbidden => ErrorKind::Forbidden,
            Self::Unauthorized | Self::InvalidToken => ErrorKind::Unauthorized,
            Self::CsrfRejected => ErrorKind::CsrfRejected,
            Self::BadRequest(_) | Self::InvalidParam(_) | Self::ParseError(_) => {
                ErrorKind::InvalidRequest
            }
            Self::Conflict(_) | Self::JobLeaseLost => ErrorKind::Conflict,
            Self::FfmpegError(_) => ErrorKind::MediaProcessingFailed,
            Self::MediaInspectionFailed(_) => ErrorKind::MediaInspectionFailed,
            Self::MediaToolUnavailable(_) => ErrorKind::MediaToolsUnavailable,
            Self::BotUnavailable(_) => ErrorKind::BotUnavailable,
            Self::UserJobLimitReached => ErrorKind::UserJobLimitReached,
            Self::ExportQueueFull => ErrorKind::ExportQueueFull,
            Self::ExecutionTimedOut => ErrorKind::ExecutionTimedOut,
            Self::WorkerInterrupted(_) => ErrorKind::WorkerInterrupted,
            Self::SourceChanged => ErrorKind::SourceChanged,
            Self::DestinationChanged => ErrorKind::DestinationChanged,
            Self::StorageBudgetExceeded => ErrorKind::StorageBudgetExceeded,
            Self::MediaArchiveUnavailable(_) => ErrorKind::MediaTemporarilyUnavailable,
            Self::ArchiveIntegrityFailure(_) => ErrorKind::ArchiveIntegrityFailure,
            Self::DiscordUnavailable(_) => ErrorKind::DiscordUnavailable,
            Self::DiscordRateLimited { .. } => ErrorKind::DiscordRateLimited,
            Self::DiscordTimeout => ErrorKind::DiscordTimeout,
            Self::RangeNotSatisfiable { .. } => ErrorKind::RangeNotSatisfiable,
            Self::InternalError
            | Self::DbError(_)
            | Self::ReqwestError(_)
            | Self::IoError(_)
            | Self::JwtError(_)
            | Self::HttpError(_)
            | Self::GrpcError(_) => ErrorKind::InternalError,
        }
    }

    /// The message a client may see. Only variants whose payload is written
    /// for users contribute text; everything else uses the kind's message.
    pub fn public_message(&self) -> Cow<'static, str> {
        match self {
            Self::BadRequest(message) | Self::Conflict(message) => Cow::Owned(message.clone()),
            Self::InvalidParam(name) => Cow::Owned(format!("Invalid request parameter: {name}")),
            Self::ParseError(_) => Cow::Borrowed("A numeric request parameter is invalid."),
            _ => Cow::Borrowed(self.kind().default_message()),
        }
    }
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::ClipNotFound => StatusCode::NOT_FOUND,
            AppError::RoleNotFound => StatusCode::NOT_FOUND,
            AppError::FileNotFound => StatusCode::NOT_FOUND,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden | AppError::CsrfRejected => StatusCode::FORBIDDEN,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::InvalidToken => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) | AppError::StorageBudgetExceeded => StatusCode::BAD_REQUEST,
            AppError::Conflict(_)
            | AppError::JobLeaseLost
            | AppError::SourceChanged
            | AppError::DestinationChanged => StatusCode::CONFLICT,
            AppError::InvalidParam(_) => StatusCode::BAD_REQUEST,
            AppError::UserJobLimitReached
            | AppError::ExportQueueFull
            | AppError::ExecutionTimedOut
            | AppError::WorkerInterrupted(_)
            | AppError::MediaToolUnavailable(_)
            | AppError::MediaArchiveUnavailable(_)
            | AppError::ArchiveIntegrityFailure(_) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::DiscordRateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AppError::DiscordTimeout => StatusCode::GATEWAY_TIMEOUT,
            AppError::MediaInspectionFailed(_) | AppError::DiscordUnavailable(_) => {
                StatusCode::BAD_GATEWAY
            }
            AppError::RangeNotSatisfiable { .. } => StatusCode::RANGE_NOT_SATISFIABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let status_code = self.status_code();
        let kind = self.kind();
        if status_code.is_server_error() {
            tracing::error!(error = ?self, kind = kind.as_str(), "request failed");
        } else {
            tracing::debug!(error = ?self, kind = kind.as_str(), "request rejected");
        }
        let error_response = ApiError {
            code: status_code.as_u16(),
            kind,
            message: self.public_message().into_owned(),
        };
        let mut response = HttpResponse::build(status_code);
        if let AppError::RangeNotSatisfiable { total } = self {
            response.insert_header((
                actix_web::http::header::CONTENT_RANGE,
                format!("bytes */{total}"),
            ));
        }
        if let AppError::DiscordRateLimited {
            retry_after: Some(seconds),
        } = self
        {
            response.insert_header((actix_web::http::header::RETRY_AFTER, seconds.to_string()));
        }
        response.json(error_response)
    }
}

/// Extractor rejections (malformed JSON bodies, paths, or queries) follow the
/// same body contract. Parser text names internal types, so it is only logged.
pub fn extractor_configs() -> (web::JsonConfig, web::PathConfig, web::QueryConfig) {
    fn reject(what: &'static str, error: &dyn std::fmt::Display) -> actix_web::Error {
        tracing::debug!(%error, what, "request extraction failed");
        AppError::BadRequest(format!("The request {what} is invalid.")).into()
    }
    (
        web::JsonConfig::default().error_handler(|error, _| reject("body", &error)),
        web::PathConfig::default().error_handler(|error, _| reject("path", &error)),
        web::QueryConfig::default().error_handler(|error, _| reject("query", &error)),
    )
}

/// A failed background job as its status endpoint reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobError {
    /// `None` for records written before kinds were persisted, or by a newer
    /// release with a kind this one does not know.
    pub kind: Option<ErrorKind>,
    pub message: String,
}

/// Shown for job errors without a trusted kind. Their stored text predates the
/// public contract and may contain internal detail, so it is never echoed.
pub const UNCLASSIFIED_JOB_ERROR: &str =
    "The last attempt did not finish. No further detail is available.";

impl JobError {
    /// Rebuild the public error from persisted columns. The displayed text
    /// always comes from the kind; stored text only signals that an error is
    /// present, so legacy free-form messages cannot leak. Writers set both
    /// columns and claims clear both, so a kind without text is stale (left by
    /// a release that predates the column) and is ignored.
    pub fn from_columns(kind: Option<&str>, stored_message: Option<&str>) -> Option<Self> {
        stored_message?;
        let kind = kind.and_then(ErrorKind::parse);
        Some(Self {
            kind,
            message: kind
                .map_or(UNCLASSIFIED_JOB_ERROR, ErrorKind::default_message)
                .to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorKind, JobError, UNCLASSIFIED_JOB_ERROR};
    use actix_web::{body::MessageBody, error::ResponseError, http::StatusCode};

    fn body(error: &AppError) -> serde_json::Value {
        let response = error.error_response();
        let bytes = response
            .into_body()
            .try_into_bytes()
            .unwrap_or_else(|_| panic!("error bodies are not streamed"));
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn kinds_round_trip_through_their_wire_names() {
        for kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::parse(kind.as_str()), Some(*kind));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(kind.as_str().to_owned())
            );
            assert!(!kind.default_message().is_empty());
        }
        assert_eq!(ErrorKind::parse("kind_from_a_future_release"), None);
    }

    #[test]
    fn media_inspection_failures_are_bad_gateways_not_server_faults() {
        // A recording ffprobe cannot measure is an upstream media problem: the
        // client's request may be perfectly valid, so it must not look like a
        // bug in this server.
        assert_eq!(
            AppError::MediaInspectionFailed("ffprobe returned no audio duration".into())
                .status_code(),
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
    fn error_bodies_carry_code_kind_and_safe_message() {
        let cases = [
            (
                AppError::UserJobLimitReached,
                503,
                "user_job_limit_reached",
                "You already have the maximum number of active media jobs. Wait for one to finish, then try again.",
            ),
            (
                AppError::ExportQueueFull,
                503,
                "export_queue_full",
                "The export queue is full. Try again after other exports finish.",
            ),
            (
                AppError::BadRequest("Clip duration must be between 1 and 20 seconds".into()),
                400,
                "invalid_request",
                "Clip duration must be between 1 and 20 seconds",
            ),
            (
                AppError::Conflict("Only finalized recordings can be deleted".into()),
                409,
                "conflict",
                "Only finalized recordings can be deleted",
            ),
            (
                AppError::Forbidden,
                403,
                "forbidden",
                "You do not have permission to do that.",
            ),
            (
                AppError::FileNotFound,
                404,
                "media_not_found",
                "The recording or media file was not found. It may have been deleted.",
            ),
            (
                AppError::DiscordTimeout,
                504,
                "discord_timeout",
                "Discord did not respond in time. Try again.",
            ),
        ];
        for (error, code, kind, message) in cases {
            assert_eq!(
                body(&error),
                serde_json::json!({ "code": code, "kind": kind, "message": message }),
                "{error:?}"
            );
        }
    }

    #[test]
    fn internal_detail_never_reaches_the_body() {
        let secret = "SELECT secret FROM /srv/sakiot/data/recordings stderr: libopus";
        let leaky = [
            AppError::DbError(sqlx::Error::Protocol(secret.into())),
            AppError::IoError(std::io::Error::other(secret)),
            AppError::FfmpegError(secret.into()),
            AppError::MediaInspectionFailed(secret.into()),
            AppError::BotUnavailable(secret.into()),
            AppError::GrpcError(secret.into()),
            AppError::MediaArchiveUnavailable(secret.into()),
            AppError::ArchiveIntegrityFailure(secret.into()),
            AppError::DiscordUnavailable(secret.into()),
        ];
        for error in leaky {
            let text = body(&error).to_string();
            assert!(!text.contains("secret"), "{error:?} leaked: {text}");
            assert!(!text.contains("/srv"), "{error:?} leaked: {text}");
            assert!(!text.contains("Database"), "{error:?} leaked: {text}");
        }
        assert_eq!(
            body(&AppError::DbError(sqlx::Error::RowNotFound)),
            serde_json::json!({
                "code": 500,
                "kind": "internal_error",
                "message": "Something went wrong on the server.",
            })
        );
        assert_eq!(
            body(&AppError::ArchiveIntegrityFailure(secret.into()))["kind"],
            "archive_integrity_failure"
        );
    }

    #[test]
    fn job_errors_never_echo_unclassified_stored_text() {
        let legacy = JobError::from_columns(None, Some("Database Error: /srv/sakiot/x"));
        assert_eq!(
            legacy,
            Some(JobError {
                kind: None,
                message: UNCLASSIFIED_JOB_ERROR.into()
            })
        );
        let future = JobError::from_columns(Some("kind_from_a_future_release"), Some("text"));
        assert_eq!(
            future,
            Some(JobError {
                kind: None,
                message: UNCLASSIFIED_JOB_ERROR.into()
            })
        );
        let known = JobError::from_columns(Some("execution_timed_out"), Some("ignored text"));
        assert_eq!(
            known,
            Some(JobError {
                kind: Some(ErrorKind::ExecutionTimedOut),
                message: ErrorKind::ExecutionTimedOut.default_message().into()
            })
        );
        assert_eq!(JobError::from_columns(None, None), None);
        // A stale kind whose text an older release already cleared.
        assert_eq!(
            JobError::from_columns(Some("execution_timed_out"), None),
            None
        );
    }

    #[actix_rt::test]
    async fn malformed_requests_use_the_contract_without_parser_detail() {
        use actix_web::{App, test, web};

        #[derive(serde::Deserialize)]
        struct Body {
            #[allow(dead_code)]
            start: f64,
        }
        let (json, path, query) = super::extractor_configs();
        let app = test::init_service(
            App::new()
                .app_data(json)
                .app_data(path)
                .app_data(query)
                .route(
                    "/{id}",
                    web::post().to(|_: web::Path<i64>, _: web::Json<Body>| async { "ok" }),
                ),
        )
        .await;
        for (uri, payload) in [
            ("/1", r#"{"start":"soon"}"#),
            ("/not-a-number", r#"{"start":1}"#),
        ] {
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri(uri)
                    .insert_header(("Content-Type", "application/json"))
                    .set_payload(payload)
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(body["kind"], "invalid_request");
            let message = body["message"].as_str().unwrap_or_default();
            assert!(
                !message.contains("f64") && !message.contains("invalid type"),
                "{message}"
            );
        }
    }
}

use std::collections::HashMap;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use actix_web::{HttpRequest, HttpResponse, post, web};
use sqlx::{Pool, Postgres};
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::auth::{Access, Token};
use crate::errors::AppError;
use crate::permissions::require_channel_access;

use super::paths::{NO_SILENCE_PREFIX, no_silence_recording_path, recording_path};
use super::util::{file_exists, get_file_path_root, handle_idempotency_key, is_stale};

const IDEMPOTENCY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_FFMPEG_ERROR_BYTES: usize = 4096;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RemoveSilenceResponse {
    pub url: String,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SilenceJobFailure {
    Ffmpeg(String),
    Database,
    Io(String),
}

impl SilenceJobFailure {
    fn into_app_error(self) -> AppError {
        match self {
            Self::Ffmpeg(message) => AppError::FfmpegError(message),
            Self::Database => AppError::InternalError,
            Self::Io(message) => AppError::IoError(std::io::Error::other(message)),
        }
    }
}

type SilenceJobResult = Result<(), SilenceJobFailure>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyRequestKey {
    user_id: i64,
    key: String,
}

#[derive(Debug)]
enum IdempotencyRecord {
    InProgress {
        fingerprint: String,
    },
    Completed {
        fingerprint: String,
        result: SilenceJobResult,
        completed_at: Instant,
    },
}

impl IdempotencyRecord {
    fn fingerprint(&self) -> &str {
        match self {
            Self::InProgress { fingerprint } | Self::Completed { fingerprint, .. } => fingerprint,
        }
    }
}

#[derive(Debug, Default)]
struct SilenceJobState {
    active: HashMap<String, broadcast::Sender<SilenceJobResult>>,
    requests: HashMap<IdempotencyRequestKey, IdempotencyRecord>,
}

#[derive(Debug, Default)]
pub struct SilenceJobContainer {
    state: Mutex<SilenceJobState>,
}

enum SilenceJobClaim {
    Start(broadcast::Sender<SilenceJobResult>),
    Wait(broadcast::Receiver<SilenceJobResult>),
    Replay(SilenceJobResult),
}

impl SilenceJobContainer {
    async fn claim(
        &self,
        user_id: i64,
        idempotency_key: String,
        fingerprint: &str,
    ) -> Result<SilenceJobClaim, AppError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        state.requests.retain(|_, record| {
            !matches!(
                record,
                IdempotencyRecord::Completed { completed_at, .. }
                    if now.duration_since(*completed_at) >= IDEMPOTENCY_TTL
            )
        });

        let request_key = IdempotencyRequestKey {
            user_id,
            key: idempotency_key,
        };
        if let Some(record) = state.requests.get(&request_key) {
            if record.fingerprint() != fingerprint {
                return Err(AppError::Conflict(
                    "Idempotency key was already used for another request".into(),
                ));
            }

            return match record {
                IdempotencyRecord::InProgress { .. } => state
                    .active
                    .get(fingerprint)
                    .map(|sender| SilenceJobClaim::Wait(sender.subscribe()))
                    .ok_or_else(|| {
                        AppError::ServiceUnavailable("processing state unavailable; retry".into())
                    }),
                IdempotencyRecord::Completed { result, .. } => {
                    Ok(SilenceJobClaim::Replay(result.clone()))
                }
            };
        }

        if let Some(sender) = state.active.get(fingerprint).cloned() {
            let receiver = sender.subscribe();
            state.requests.insert(
                request_key,
                IdempotencyRecord::InProgress {
                    fingerprint: fingerprint.to_owned(),
                },
            );
            return Ok(SilenceJobClaim::Wait(receiver));
        }

        let (sender, _) = broadcast::channel(1);
        state.active.insert(fingerprint.to_owned(), sender.clone());
        state.requests.insert(
            request_key,
            IdempotencyRecord::InProgress {
                fingerprint: fingerprint.to_owned(),
            },
        );
        Ok(SilenceJobClaim::Start(sender))
    }

    async fn finish(
        &self,
        fingerprint: &str,
        sender: &broadcast::Sender<SilenceJobResult>,
        result: SilenceJobResult,
    ) {
        let mut state = self.state.lock().await;
        state.active.remove(fingerprint);
        let completed_at = Instant::now();
        for record in state.requests.values_mut() {
            if matches!(
                record,
                IdempotencyRecord::InProgress {
                    fingerprint: active_fingerprint
                } if active_fingerprint == fingerprint
            ) {
                *record = IdempotencyRecord::Completed {
                    fingerprint: fingerprint.to_owned(),
                    result: result.clone(),
                    completed_at,
                };
            }
        }
        drop(state);

        let _ = sender.send(result);
    }
}

fn recording_fingerprint(path: &(i64, i64, i32, i32, String)) -> String {
    format!(
        "{}/{}/{:04}/{:02}/{}",
        path.0, path.1, path.2, path.3, path.4
    )
}

fn ffmpeg_result(output: &Output) -> SilenceJobResult {
    if output.status.success() {
        return Ok(());
    }

    let stderr = &output.stderr[..output.stderr.len().min(MAX_FFMPEG_ERROR_BYTES)];
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    Err(SilenceJobFailure::Ffmpeg(if message.is_empty() {
        format!("ffmpeg exited with {}", output.status)
    } else {
        message
    }))
}

async fn wait_for_existing_job(
    mut receiver: broadcast::Receiver<SilenceJobResult>,
    file_no_silence: String,
) -> Result<HttpResponse, AppError> {
    match receiver.recv().await {
        Ok(Ok(())) => Ok(HttpResponse::Accepted().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: "Success",
        })),
        Ok(Err(failure)) => Err(failure.into_app_error()),
        Err(error) => {
            error!(?error, "silence removal result channel failed");
            Err(AppError::ServiceUnavailable(
                "processing result unavailable; retry".into(),
            ))
        }
    }
}

fn replay_job_result(
    result: SilenceJobResult,
    file_no_silence: String,
) -> Result<HttpResponse, AppError> {
    result
        .map(|()| {
            HttpResponse::Ok().json(RemoveSilenceResponse {
                url: file_no_silence,
                message: "Request already completed",
            })
        })
        .map_err(SilenceJobFailure::into_app_error)
}

#[utoipa::path(
    post,
    path = "/api/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}",
    tag = "audio",
    params(
        ("guild_id" = i64, Path, description = "Discord guild id"),
        ("channel_id" = i64, Path, description = "Discord channel id"),
        ("year" = i32, Path, description = "Recording year"),
        ("month" = i32, Path, description = "Recording month"),
        ("file_name" = String, Path, description = "Recording file stem"),
        ("Idempotency-Key" = String, Header, description = "Idempotency key for processing request"),
    ),
    responses(
        (status = 200, description = "Silence removal file exists or processing started", body = RemoveSilenceResponse),
        (status = 202, description = "Existing processing request completed", body = RemoveSilenceResponse),
        (status = 400, description = "Missing idempotency key", body = crate::errors::ApiError),
        (status = 401, description = "Missing or invalid access token", body = crate::errors::ApiError),
        (status = 403, description = "Missing channel permission", body = crate::errors::ApiError),
        (status = 409, description = "Idempotency key reused for another request", body = crate::errors::ApiError),
        (status = 503, description = "Concurrent processing state unavailable", body = crate::errors::ApiError),
        (status = 500, description = "Server error", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[post("/remove_silence/{guild_id}/{channel_id}/{year}/{month}/{file_name}")]
pub async fn remove_silence(
    req: HttpRequest,
    path: web::Path<(i64, i64, i32, i32, String)>,
    jobs: web::Data<SilenceJobContainer>,
    pool: web::Data<Pool<Postgres>>,
    token: Option<web::ReqData<Token<Access>>>,
) -> Result<HttpResponse, AppError> {
    let path = path.into_inner();
    let token = token.ok_or(AppError::Unauthorized)?;
    require_channel_access(&pool, path.0, path.1, token.user_id).await?;

    let file_path: String = get_file_path_root(&recording_path(), &path);
    let no_silence_file_path = get_file_path_root(&no_silence_recording_path(), &path);
    let file_no_silence =
        no_silence_file_path.to_owned() + "/" + NO_SILENCE_PREFIX + path.4.as_str() + ".ogg";

    let idempotency_key = handle_idempotency_key(&req)?;
    let fingerprint = recording_fingerprint(&path);
    let claim = jobs
        .claim(token.user_id, idempotency_key, &fingerprint)
        .await?;

    info!("File name: {}", path.4);

    // Source recording and the (correctly prefixed) cached output. The cache is
    // only valid when it's newer than the source — a file produced from an
    // earlier, shorter version of the recording (e.g. while still live) is
    // treated as stale, so the user can refresh it as the recording grows.
    let source_file = format!("{}/{}.ogg", file_path, path.4);
    let cached_fresh =
        file_exists(&file_no_silence).await && !is_stale(&source_file, &file_no_silence).await;

    let sender = match claim {
        SilenceJobClaim::Wait(receiver) => {
            info!("silence removal already processing");
            return wait_for_existing_job(receiver, file_no_silence).await;
        }
        SilenceJobClaim::Replay(result) => {
            return replay_job_result(result, file_no_silence);
        }
        SilenceJobClaim::Start(sender) => sender,
    };

    if cached_fresh {
        info!("silence-free file already exists");
        jobs.finish(&fingerprint, &sender, Ok(())).await;
        return Ok(HttpResponse::Ok().json(RemoveSilenceResponse {
            url: file_no_silence,
            message: "File already exists",
        }));
    }

    info!("Creating new file");
    if let Err(err) = tokio::fs::create_dir_all(&no_silence_file_path).await {
        error!("create_dir_all failed: {}", err);
        jobs.finish(
            &fingerprint,
            &sender,
            Err(SilenceJobFailure::Io(err.to_string())),
        )
        .await;
        return Err(AppError::IoError(err));
    }

    let file = source_file.clone();
    let file_no_silence_clone = file_no_silence.to_owned();

    info!("NO SILENCE FILE PATH: {}", &file_no_silence);

    let jobs_clone = jobs.clone();
    tokio::spawn(async move {
        let file_name: String = path.4.clone();
        let mut command = tokio::process::Command::new("ffmpeg");
        command
            // -y so a stale cached file is overwritten on regeneration
            // (otherwise ffmpeg blocks on an interactive overwrite prompt).
            .arg("-y")
            .args(["-i", &file])
            .args([
                "-af",
                "silenceremove=stop_periods=-1:stop_duration=1:stop_threshold=-40dB",
            ])
            .arg(&file_no_silence_clone)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let result = match command.output().await {
            Ok(output) => ffmpeg_result(&output),
            Err(err) => Err(SilenceJobFailure::Ffmpeg(format!(
                "could not start ffmpeg: {err}"
            ))),
        };

        let result = match result {
            Ok(()) => {
                if let Err(error) = sqlx::query!(
                    "UPDATE public.audio_files
				SET silence=true
				WHERE file_name=$1;",
                    file_name
                )
                .execute(pool.get_ref())
                .await
                {
                    error!(?error, "audio_files silence update failed");
                    let _ = tokio::fs::remove_file(&file_no_silence_clone).await;
                    Err(SilenceJobFailure::Database)
                } else {
                    Ok(())
                }
            }
            Err(failure) => {
                error!(?failure, "ffmpeg silence removal failed");
                let _ = tokio::fs::remove_file(&file_no_silence_clone).await;
                Err(failure)
            }
        };

        jobs_clone.finish(&fingerprint, &sender, result).await;
    });

    Ok(HttpResponse::Ok().json(RemoveSilenceResponse {
        url: file_no_silence,
        message: "Request Accepted",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    #[tokio::test]
    async fn idempotency_replays_completed_result() -> Result<(), Box<dyn std::error::Error>> {
        let jobs = SilenceJobContainer::default();
        let fingerprint = "1/2/2026/06/recording";
        let sender = match jobs.claim(7, "request-1".into(), fingerprint).await? {
            SilenceJobClaim::Start(sender) => sender,
            _ => return Err("first request should start job".into()),
        };

        jobs.finish(fingerprint, &sender, Ok(())).await;

        assert!(matches!(
            jobs.claim(7, "request-1".into(), fingerprint).await?,
            SilenceJobClaim::Replay(Ok(()))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn idempotency_rejects_key_reuse_for_different_recording()
    -> Result<(), Box<dyn std::error::Error>> {
        let jobs = SilenceJobContainer::default();
        assert!(matches!(
            jobs.claim(7, "request-1".into(), "recording-a").await?,
            SilenceJobClaim::Start(_)
        ));

        assert!(matches!(
            jobs.claim(7, "request-1".into(), "recording-b").await,
            Err(AppError::Conflict(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_keys_share_same_recording_job() -> Result<(), Box<dyn std::error::Error>> {
        let jobs = SilenceJobContainer::default();
        assert!(matches!(
            jobs.claim(7, "request-1".into(), "recording-a").await?,
            SilenceJobClaim::Start(_)
        ));
        assert!(matches!(
            jobs.claim(7, "request-2".into(), "recording-a").await?,
            SilenceJobClaim::Wait(_)
        ));
        Ok(())
    }

    #[test]
    fn ffmpeg_nonzero_exit_is_failure() {
        let output = Output {
            status: exit_status(1),
            stdout: Vec::new(),
            stderr: b"invalid input".to_vec(),
        };

        assert_eq!(
            ffmpeg_result(&output),
            Err(SilenceJobFailure::Ffmpeg("invalid input".into()))
        );
    }
}

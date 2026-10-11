use std::collections::HashMap;

use sakiot_paths::DataRoots;
use sakiot_peaks::{FRAME_MS, FramePeak};

use super::*;
use crate::media_jobs::JobAttempt;
use crate::waveform::{
    PeakDensity, decode_and_publish, frames_per_point, read_peaks, waveform_dat,
};

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/waveform",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Combined peaks of the session's recordings, live ones included; explicit gaps are zero-valued", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/waveform")]
pub async fn get_session_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let viewer = crate::permissions::Viewer::of(&token);
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, viewer).await?;
    let resource = session_waveform_resource(session_id, false);
    if let Some(job) =
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_waveform", &resource)
            .await?
    {
        return Ok(building_response(job));
    }
    let parts = timeline_layout(&pool, &access).await?;
    let peaks = fragment_peaks(&parts).await?;
    // An ended recording without peaks is decoded for them. A live one stays
    // blank until it ends: decoding it would leave a snapshot that stops short.
    if let Some(missing) = parts.iter().find_map(|part| match &part.kind {
        PartKind::Audio {
            audio_file_id,
            file_name,
            live: false,
            ..
        } if !peaks.contains_key(file_name) => Some(*audio_file_id),
        _ => None,
    }) {
        return build_session_waveform(&pool, viewer, access.guild_id, session_id, false, missing)
            .await;
    }
    let growing = finished_at_ms(&access).is_none();
    let spans = spans(access.started_at_ms, &parts, &peaks, growing);
    Ok(dat_response(session_dat(&spans, growing)))
}

/// The peaks of every recording in `parts` that has them, by stem.
async fn fragment_peaks(
    parts: &[TimelinePart],
) -> Result<HashMap<String, Vec<FramePeak>>, AppError> {
    let roots = DataRoots::from_env();
    let mut peaks = HashMap::new();
    for part in parts {
        if let PartKind::Audio { file_name, .. } = &part.kind
            && !peaks.contains_key(file_name)
            && let Some(frames) = read_peaks(&roots.peaks_path(file_name)).await?
        {
            peaks.insert(file_name.clone(), frames);
        }
    }
    Ok(peaks)
}

/// One stretch of a session's timeline: the recording's peaks heard there,
/// then silence to `frames`.
#[derive(Debug)]
struct Span<'a> {
    heard: &'a [FramePeak],
    frames: usize,
}

/// The session's timeline as spans: each recording's peaks where it is
/// heard, silence for gaps and for recordings without peaks. A growing
/// session ends where its last recording's peaks end, as its audio does.
fn spans<'a>(
    started_at_ms: i64,
    parts: &[TimelinePart],
    peaks: &'a HashMap<String, Vec<FramePeak>>,
    growing: bool,
) -> Vec<Span<'a>> {
    let frame_at =
        |ms: i64| usize::try_from(ms.saturating_sub(started_at_ms) / FRAME_MS).unwrap_or(0);
    let mut spans = Vec::with_capacity(parts.len());
    let mut cursor = 0;
    for (index, part) in parts.iter().enumerate() {
        let start = frame_at(part.start_ms).max(cursor);
        let end = frame_at(part.end_ms).max(start);
        if start > cursor {
            spans.push(Span {
                heard: &[],
                frames: start - cursor,
            });
        }
        let heard = match &part.kind {
            PartKind::Audio {
                source_start_ms,
                file_name,
                ..
            } => {
                let offset =
                    usize::try_from(part.start_ms.saturating_sub(*source_start_ms) / FRAME_MS)
                        .unwrap_or(0);
                let recording = peaks
                    .get(file_name)
                    .and_then(|recording| recording.get(offset..))
                    .unwrap_or_default();
                if growing && index + 1 == parts.len() {
                    spans.push(Span {
                        heard: recording,
                        frames: recording.len(),
                    });
                    break;
                }
                recording.get(..end - start).unwrap_or(recording)
            }
            PartKind::Silence => &[],
        };
        spans.push(Span {
            heard,
            frames: end - start,
        });
        cursor = end;
    }
    spans
}

/// The session's waveform, condensed as its spans are walked, so a long gap
/// (or a session left "active" for weeks) costs one step per point.
fn session_dat(spans: &[Span<'_>], growing: bool) -> Vec<u8> {
    let total = spans.iter().map(|span| span.frames).sum();
    let per_point = frames_per_point(
        total,
        PeakDensity::PerSecond(SESSION_PEAKS_PER_SECOND),
        growing,
    );
    let mut condenser = sakiot_peaks::Condenser::new(per_point);
    for span in spans {
        condenser.extend(span.heard);
        condenser.push_silence(span.frames - span.heard.len());
    }
    sakiot_peaks::encode_dat(&condenser.finish(), per_point)
}

fn dat_response(dat: Vec<u8>) -> HttpResponse {
    HttpResponse::Ok().json(SessionWaveformResponse {
        progress: 100,
        building: false,
        data: Some(BASE64_STANDARD.encode(dat)),
        job_id: None,
        error: None,
    })
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/waveform/rebuild",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Decoding of the ended recordings without peaks started or already running", body = SessionWaveformResponse),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Session not found", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/waveform/rebuild")]
pub async fn rebuild_session_waveform(
    request: HttpRequest,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    enqueue_session_waveform(
        &request,
        &pool,
        crate::permissions::Viewer::of(&token),
        access.guild_id,
        session_id,
        false,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/audio/sessions/{recording_session_id}/silence-free/waveform",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Silence-free session waveform status and peaks", body = SessionWaveformResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Silence-free session has not been generated", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = [])),
)]
#[get("/audio/sessions/{recording_session_id}/silence-free/waveform")]
pub async fn get_session_silence_free_waveform(
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let viewer = crate::permissions::Viewer::of(&token);
    let session_id = path.into_inner();
    let access = require_session_access(&pool, session_id, viewer).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    let resource = session_waveform_resource(session_id, true);
    if let Some(job) =
        crate::media_jobs::active_for_resource(&pool, token.user_id, "session_waveform", &resource)
            .await?
    {
        return Ok(building_response(job));
    }
    // Silence-free audio exists only once the session has finished. Its
    // peaks go stale when it is rendered again.
    let peaks = silence_free_peaks_path(session_id);
    if !waveform_is_stale(&peaks, &source).await
        && let Some(frames) = read_peaks(&peaks).await?
    {
        return Ok(dat_response(waveform_dat(
            &frames,
            PeakDensity::PerSecond(SESSION_PEAKS_PER_SECOND),
            false,
        )));
    }
    build_session_waveform(
        &pool,
        viewer,
        access.guild_id,
        session_id,
        true,
        modified_ms(&source).await.unwrap_or_default(),
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/audio/sessions/{recording_session_id}/silence-free/waveform/rebuild",
    tag = "audio",
    params(("recording_session_id" = i64, Path, description = "Logical recording session id")),
    responses(
        (status = 200, description = "Silence-free waveform rebuild started or already running", body = SessionWaveformResponse),
        (status = 400, description = "Session is not finalized", body = crate::errors::ApiError),
        (status = 401, description = "Missing access token", body = crate::errors::ApiError),
        (status = 403, description = "One or more audible channels inaccessible", body = crate::errors::ApiError),
        (status = 404, description = "Silence-free session has not been generated", body = crate::errors::ApiError),
        (status = 500, description = "Waveform generation failed", body = crate::errors::ApiError),
    ),
    security(("access_token" = []), ("csrf_token" = [])),
)]
#[post("/audio/sessions/{recording_session_id}/silence-free/waveform/rebuild")]
pub async fn rebuild_session_silence_free_waveform(
    request: HttpRequest,
    path: web::Path<i64>,
    token: Option<web::ReqData<Token<Access>>>,
    pool: web::Data<Pool<Postgres>>,
) -> Result<HttpResponse, AppError> {
    let token = token.ok_or(AppError::Unauthorized)?;
    let session_id = path.into_inner();
    let access =
        require_session_access(&pool, session_id, crate::permissions::Viewer::of(&token)).await?;
    let source = session_silence_free_path(&access)?;
    if !tokio::fs::try_exists(&source).await? {
        return Err(AppError::FileNotFound);
    }
    enqueue_session_waveform(
        &request,
        &pool,
        crate::permissions::Viewer::of(&token),
        access.guild_id,
        session_id,
        true,
    )
    .await
}

async fn enqueue_session_waveform(
    http_request: &HttpRequest,
    pool: &web::Data<Pool<Postgres>>,
    requester: crate::permissions::Viewer,
    guild_id: i64,
    session_id: i64,
    silence_free: bool,
) -> Result<HttpResponse, AppError> {
    let resource = session_waveform_resource(session_id, silence_free);
    let request = crate::media_jobs::MediaJobRequest::SessionWaveform {
        session_id,
        silence_free,
    };
    let key = http_request
        .headers()
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let status =
        crate::media_jobs::enqueue(pool, Some(guild_id), requester, &key, &resource, &request)
            .await?;
    Ok(HttpResponse::Accepted()
        .insert_header((
            actix_web::http::header::LOCATION,
            format!("/api/media-jobs/{}", status.id),
        ))
        .json(status))
}

fn building_response(job: crate::media_jobs::MediaJobStatus) -> HttpResponse {
    HttpResponse::Ok().json(SessionWaveformResponse {
        progress: job.progress,
        building: true,
        data: None,
        job_id: Some(job.id),
        error: None,
    })
}

/// When a finalized session ended; `None` while it can still grow.
fn finished_at_ms(access: &SessionAccess) -> Option<i64> {
    (access.state == "finalized").then(|| access.ended_at_ms.unwrap_or(access.started_at_ms))
}

/// A file's modification time in Unix milliseconds.
async fn modified_ms(path: &Path) -> Option<i64> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

/// Starts building a session waveform's missing peaks without being asked.
/// The job key names what it builds (`covers`), so repeated requests share
/// one job and a failed build is reported instead of retried on every
/// request.
async fn build_session_waveform(
    pool: &web::Data<Pool<Postgres>>,
    requester: crate::permissions::Viewer,
    guild_id: i64,
    session_id: i64,
    silence_free: bool,
    covers: i64,
) -> Result<HttpResponse, AppError> {
    let resource = session_waveform_resource(session_id, silence_free);
    let request = crate::media_jobs::MediaJobRequest::SessionWaveform {
        session_id,
        silence_free,
    };
    let key = format!("{resource}-{covers}");
    let job = crate::media_jobs::enqueue_rebuild(
        pool,
        Some(guild_id),
        requester,
        &key,
        &resource,
        &request,
    )
    .await?;
    if job.status == "failed" {
        return Ok(HttpResponse::Ok().json(SessionWaveformResponse {
            progress: 0,
            building: false,
            data: None,
            job_id: None,
            error: Some(
                job.error
                    .unwrap_or_else(|| "The waveform could not be built.".to_owned()),
            ),
        }));
    }
    Ok(building_response(job))
}

/// Decodes peaks for a session: its ended recordings that have none, or its
/// silence-free render.
pub(crate) async fn run_session_waveform_job(
    attempt: &JobAttempt<'_>,
    session_id: i64,
    silence_free: bool,
) -> Result<(Option<String>, Option<PathBuf>), AppError> {
    let pool = web::Data::new(attempt.pool.clone());
    let access = require_session_access(&pool, session_id, attempt.requester).await?;
    let sources = if silence_free {
        let source = session_silence_free_path(&access)?;
        if !tokio::fs::try_exists(&source).await? {
            return Err(AppError::FileNotFound);
        }
        vec![(source, silence_free_peaks_path(session_id))]
    } else {
        let roots = DataRoots::from_env();
        let mut sources = Vec::new();
        for part in timeline_layout(&pool, &access).await? {
            let PartKind::Audio {
                path,
                audio_file_id,
                file_name,
                live: false,
                ..
            } = part.kind
            else {
                continue;
            };
            let peaks = roots.peaks_path(&file_name);
            if read_peaks(&peaks).await?.is_none() {
                attempt
                    .media
                    .ensure_recording_local(attempt.pool, audio_file_id, &path)
                    .await?;
                sources.push((path, peaks));
            }
        }
        sources
    };
    let route = if silence_free {
        format!("/api/audio/sessions/{session_id}/silence-free/waveform")
    } else {
        format!("/api/audio/sessions/{session_id}/waveform")
    };
    decode_and_publish(attempt, &sources, &route).await?;
    Ok((Some(route), None))
}

/// The media-job resource of a session waveform.
pub(super) fn session_waveform_resource(session_id: i64, silence_free: bool) -> String {
    let suffix = if silence_free { "-silence-free" } else { "" };
    format!("logical-session-{session_id}{suffix}")
}

/// Where a session's silence-free render keeps its peaks. Its normal waveform
/// has no file: it is stitched from its recordings' peaks.
pub(super) fn silence_free_peaks_path(session_id: i64) -> PathBuf {
    DataRoots::from_env().peaks_path(&session_waveform_resource(session_id, true))
}

pub(super) async fn waveform_is_stale(waveform: &Path, source: &Path) -> bool {
    let Ok(waveform_modified) = tokio::fs::metadata(waveform)
        .await
        .and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    let Ok(source_modified) = tokio::fs::metadata(source)
        .await
        .and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    waveform_modified < source_modified
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOUD: FramePeak = FramePeak { min: -50, max: 60 };

    fn audio(start_ms: i64, end_ms: i64, source_start_ms: i64, file_name: &str) -> TimelinePart {
        TimelinePart {
            start_ms,
            end_ms,
            kind: PartKind::Audio {
                path: PathBuf::new(),
                source_start_ms,
                audio_file_id: 1,
                file_name: file_name.to_owned(),
                live: false,
            },
        }
    }

    fn silence(start_ms: i64, end_ms: i64) -> TimelinePart {
        TimelinePart {
            start_ms,
            end_ms,
            kind: PartKind::Silence,
        }
    }

    fn peaks(entries: &[(&str, Vec<FramePeak>)]) -> HashMap<String, Vec<FramePeak>> {
        entries
            .iter()
            .map(|(name, frames)| ((*name).to_owned(), frames.clone()))
            .collect()
    }

    /// The frames `spans` stand for, one by one.
    fn stitch(
        started_at_ms: i64,
        parts: &[TimelinePart],
        peaks: &HashMap<String, Vec<FramePeak>>,
        growing: bool,
    ) -> Vec<FramePeak> {
        spans(started_at_ms, parts, peaks, growing)
            .iter()
            .flat_map(|span| {
                span.heard.iter().copied().chain(std::iter::repeat_n(
                    FramePeak::SILENT,
                    span.frames - span.heard.len(),
                ))
            })
            .collect()
    }

    #[test]
    fn stitching_places_each_recording_on_the_session_timeline() {
        // Session from 1,000 ms: 40 ms of silence, then recording "a" from
        // 60 ms into its file, then a gap, then "b".
        let a: Vec<FramePeak> = (0..10)
            .map(|frame| FramePeak {
                min: -frame,
                max: frame,
            })
            .collect();
        let parts = [
            silence(1_000, 1_040),
            audio(1_040, 1_100, 980, "a"),
            silence(1_100, 1_140),
            audio(1_140, 1_180, 1_140, "b"),
        ];
        let frames = stitch(
            1_000,
            &parts,
            &peaks(&[("a", a.clone()), ("b", vec![LOUD; 5])]),
            false,
        );
        let silent = FramePeak::SILENT;
        assert_eq!(
            frames,
            [silent, silent, a[3], a[4], a[5], silent, silent, LOUD, LOUD]
        );
    }

    #[test]
    fn missing_or_short_peaks_are_silent_until_the_part_ends() {
        let parts = [audio(0, 100, 0, "short"), audio(100, 160, 100, "gone")];
        let frames = stitch(0, &parts, &peaks(&[("short", vec![LOUD; 2])]), false);
        assert_eq!(frames.len(), 8);
        assert_eq!(frames[..2], [LOUD; 2]);
        assert!(frames[2..].iter().all(|frame| *frame == FramePeak::SILENT));
    }

    #[test]
    fn a_growing_session_ends_where_its_last_recording_does() {
        let parts = [silence(0, 40), audio(40, 400, 40, "live")];
        let frames = stitch(0, &parts, &peaks(&[("live", vec![LOUD; 3])]), true);
        assert_eq!(frames.len(), 5);
        // An ended session is padded to its full length.
        let frames = stitch(0, &parts, &peaks(&[("live", vec![LOUD; 3])]), false);
        assert_eq!(frames.len(), 20);
    }

    #[test]
    fn a_session_left_active_for_months_condenses_its_silence_cheaply() {
        // 150 days of silence after a minute of audio.
        let day_ms = 86_400_000;
        let parts = [audio(0, 60_000, 0, "a"), silence(60_000, 150 * day_ms)];
        let peaks = peaks(&[("a", vec![LOUD; 3_000])]);
        let spans = spans(0, &parts, &peaks, true);
        assert_eq!(
            spans.iter().map(|span| span.frames).sum::<usize>(),
            648_000_000
        );
        let dat = session_dat(&spans, true);
        // Capped at 60,000 points, grouped by a power of two: 16,384 frames.
        assert_eq!(&dat[12..16], &(16_384u32 * 960).to_le_bytes());
        assert_eq!(
            &dat[20..22],
            &[LOUD.min.to_le_bytes()[0], LOUD.max.to_le_bytes()[0]]
        );
    }
}

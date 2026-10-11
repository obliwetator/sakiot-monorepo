//! Waveforms condensed from peaks files, and peaks files built by decoding
//! audio the recorder wrote none for.
//!
//! The recorder writes a recording's peaks while it records, so serving a
//! waveform, live or finished, reads a small file and decodes nothing. Audio
//! without peaks (silence-free renders, clips, recordings whose peaks were
//! evicted or never written) is decoded once by a media job.

use std::path::{Path, PathBuf};

use actix_web::web;
use sakiot_peaks::{FRAME_MS, FramePeak};

use crate::audio::WaveformProgressContainer;
use crate::errors::AppError;
use crate::media_jobs::JobAttempt;

const DEFAULT_TARGET_POINTS: f64 = 2500.0;
const MIN_TARGET_POINTS: f64 = 2500.0;
/// Above this the payload costs more than the extra detail is worth: 60k points
/// is ~120 KB of 8-bit min/max pairs before base64.
const MAX_TARGET_POINTS: f64 = 60_000.0;

/// How finely a waveform is sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PeakDensity {
    /// A fixed number of points across the file, however long it is.
    Fixed(f64),
    /// Points per second, so a long recording still resolves detail when the
    /// dashboard zooms into it. Clamped at both ends.
    PerSecond(f64),
}

impl PeakDensity {
    pub const DEFAULT: Self = Self::Fixed(DEFAULT_TARGET_POINTS);

    fn target_points(self, duration_seconds: f64) -> f64 {
        match self {
            Self::Fixed(points) => points,
            Self::PerSecond(rate) => {
                (duration_seconds * rate).clamp(MIN_TARGET_POINTS, MAX_TARGET_POINTS)
            }
        }
    }
}

/// The audiowaveform `.dat` of `frames` at `density`. A growing (live)
/// source keeps its points in place between refreshes.
pub fn waveform_dat(frames: &[FramePeak], density: PeakDensity, growing: bool) -> Vec<u8> {
    let per_point = frames_per_point(frames.len(), density, growing);
    sakiot_peaks::encode_dat(&sakiot_peaks::condense(frames, per_point), per_point)
}

/// Frames per point of a waveform `frame_count` frames long at `density`.
pub fn frames_per_point(frame_count: usize, density: PeakDensity, growing: bool) -> usize {
    let duration_seconds = frame_count as f64 * FRAME_MS as f64 / 1_000.0;
    let target_points = density.target_points(duration_seconds).round() as usize;
    sakiot_peaks::frames_per_point(frame_count, target_points, growing)
}

/// The frames of a peaks file, or `None` when there is none to read. An
/// unreadable file also reads as none, so the rebuild that follows replaces it.
pub async fn read_peaks(path: &Path) -> Result<Option<Vec<FramePeak>>, AppError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    match sakiot_peaks::parse(&bytes) {
        Ok(frames) => Ok(Some(frames)),
        Err(error) => {
            tracing::warn!(path = %path.display(), "unreadable waveform peaks file: {error}");
            Ok(None)
        }
    }
}

/// Decodes the Ogg Opus file `input` into a peaks file at `output`,
/// reporting progress into `progress[key]`, scaled into `range` when given.
pub async fn decode_peaks(
    input: &Path,
    output: &Path,
    key: &str,
    progress: &web::Data<WaveformProgressContainer>,
    range: Option<(i16, i16)>,
) -> Result<(), AppError> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let input = input.to_path_buf();
    let key = key.to_owned();
    let progress = progress.clone();
    let frames = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&input)?;
        let size = file.metadata()?.len().max(1);
        let mut reported = None;
        sakiot_peaks::decode::decode(file, |read| {
            let percent = i16::try_from(read.saturating_mul(100) / size)
                .unwrap_or(100)
                .min(100);
            if reported != Some(percent) {
                reported = Some(percent);
                progress
                    .0
                    .blocking_write()
                    .insert(key.clone(), scale_progress(percent, range));
            }
        })
        .map_err(|error| AppError::IoError(std::io::Error::other(error.to_string())))
    })
    .await
    .map_err(|_| AppError::InternalError)??;
    tokio::fs::write(output, sakiot_peaks::encode(&frames)).await?;
    Ok(())
}

/// Decodes each `(audio, peaks)` pair into its peaks file for a media job,
/// then publishes them together under the job's lease, so an attempt that
/// lost its lease never replaces them.
pub(crate) async fn decode_and_publish(
    attempt: &JobAttempt<'_>,
    sources: &[(PathBuf, PathBuf)],
    result_url: &str,
) -> Result<(), AppError> {
    let JobAttempt {
        pool, id, token, ..
    } = *attempt;
    const KEY: &str = "peaks";
    let progress = web::Data::new(WaveformProgressContainer(Default::default()));
    let staged: Vec<(PathBuf, &Path)> = sources
        .iter()
        .map(|(_, peaks)| {
            (
                peaks.with_extension(format!("{id}.{token}.tmp.peaks")),
                peaks.as_path(),
            )
        })
        .collect();
    let count = i16::try_from(sources.len()).unwrap_or(i16::MAX).max(1);
    let published = async {
        for (index, ((audio, _), (temp, _))) in sources.iter().zip(&staged).enumerate() {
            let index = i16::try_from(index).unwrap_or(i16::MAX);
            let range = (index * 99 / count, (index + 1) * 99 / count);
            crate::media_jobs::track_progress(
                pool,
                id,
                token,
                "waveform",
                &progress,
                KEY,
                decode_peaks(audio, temp, KEY, &progress, Some(range)),
            )
            .await?;
        }
        let tx = crate::media_jobs::begin_publication(pool, id, token).await?;
        for (temp, peaks) in &staged {
            tokio::fs::rename(temp, peaks).await?;
        }
        crate::media_jobs::complete_publication(tx, id, token, result_url, None).await
    }
    .await;
    if published.is_err() {
        for (temp, _) in &staged {
            let _ = tokio::fs::remove_file(temp).await;
        }
    }
    published
}

fn scale_progress(progress: i16, range: Option<(i16, i16)>) -> i16 {
    let progress = progress.clamp(0, 100);
    let Some((start, end)) = range else {
        return progress.min(99);
    };
    let start = start.clamp(0, 99);
    let end = end.clamp(start, 99);
    start + (progress * (end - start) / 100)
}

#[cfg(test)]
mod tests {
    use super::{PeakDensity, read_peaks, scale_progress, waveform_dat};
    use sakiot_peaks::FramePeak;

    fn dat_header(dat: &[u8]) -> [u32; 5] {
        std::array::from_fn(|field| {
            u32::from_le_bytes(dat[field * 4..field * 4 + 4].try_into().unwrap())
        })
    }

    #[test]
    fn scales_peak_generation_into_reserved_progress_range() {
        assert_eq!(scale_progress(0, Some((85, 99))), 85);
        assert_eq!(scale_progress(50, Some((85, 99))), 92);
        assert_eq!(scale_progress(100, Some((85, 99))), 99);
        assert_eq!(scale_progress(100, None), 99);
    }

    #[test]
    fn fixed_density_ignores_duration() {
        assert_eq!(PeakDensity::DEFAULT.target_points(30.0), 2500.0);
        assert_eq!(PeakDensity::DEFAULT.target_points(22_484.0), 2500.0);
    }

    #[test]
    fn per_second_density_scales_between_its_bounds() {
        let density = PeakDensity::PerSecond(4.0);
        // A short recording keeps the old resolution.
        assert_eq!(density.target_points(60.0), 2500.0);
        assert_eq!(density.target_points(1_000.0), 4_000.0);
        // A six hour session caps out at ~0.37s per point.
        assert_eq!(density.target_points(22_484.0), 60_000.0);
    }

    #[test]
    fn a_finished_hour_condenses_to_about_the_target() {
        // One hour is 180,000 frames: 72 frames (1.44 s) per point.
        let frames = vec![FramePeak { min: -3, max: 4 }; 180_000];
        let dat = waveform_dat(&frames, PeakDensity::DEFAULT, false);
        assert_eq!(dat_header(&dat), [1, 1, 48_000, 72 * 960, 2_500]);
        assert_eq!(&dat[20..22], &[0xFD, 4]);
    }

    #[test]
    fn a_live_hour_groups_by_a_power_of_two() {
        let frames = vec![FramePeak::SILENT; 180_000];
        let dat = waveform_dat(&frames, PeakDensity::DEFAULT, true);
        // 128 frames per point: 1,407 points, the last one partial.
        assert_eq!(dat_header(&dat), [1, 1, 48_000, 128 * 960, 1_407]);
    }

    #[tokio::test]
    async fn missing_and_unreadable_peaks_read_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_peaks(&dir.path().join("none.peaks")).await.unwrap(),
            None
        );
        let garbage = dir.path().join("garbage.peaks");
        std::fs::write(&garbage, b"not peaks").unwrap();
        assert_eq!(read_peaks(&garbage).await.unwrap(), None);
        let good = dir.path().join("good.peaks");
        std::fs::write(&good, sakiot_peaks::encode(&[FramePeak::SILENT; 3])).unwrap();
        assert_eq!(
            read_peaks(&good).await.unwrap(),
            Some(vec![FramePeak::SILENT; 3])
        );
    }

    /// An Ogg Opus stream of `frames` stereo silence frames.
    fn silent_ogg_opus(frames: u64) -> Vec<u8> {
        use ogg::PacketWriteEndInfo::{EndPage, EndStream, NormalPacket};
        let mut bytes = Vec::new();
        let mut writer = ogg::PacketWriter::new(std::io::Cursor::new(&mut bytes));
        let mut head = b"OpusHead\x01\x02\0\0".to_vec();
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        writer.write_packet(head, 1, EndPage, 0).unwrap();
        writer
            .write_packet(b"OpusTags\0\0\0\0\0\0\0\0".to_vec(), 1, EndPage, 0)
            .unwrap();
        for frame in 1..=frames {
            let end = if frame == frames {
                EndStream
            } else {
                NormalPacket
            };
            writer
                .write_packet(vec![0xFC, 0xFF, 0xFE], 1, end, frame * 960)
                .unwrap();
        }
        drop(writer);
        bytes
    }

    /// A media job holding its lease, as the worker claims it.
    async fn running_job(pool: &sqlx::PgPool) -> crate::media_jobs::MediaJobStatus {
        let request = crate::media_jobs::MediaJobRequest::ClipWaveform {
            guild_id: 1,
            clip_id: "clip".to_owned(),
        };
        let job = crate::media_jobs::enqueue(
            pool,
            Some(1),
            crate::permissions::Viewer::discord(10),
            "key",
            "resource",
            &request,
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE media_jobs SET state='running', attempt_token='token',
                    lease_expires_at=now()+interval '1 minute'
              WHERE id=$1",
        )
        .bind(&job.id)
        .execute(pool)
        .await
        .unwrap();
        job
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn a_decode_job_publishes_the_peaks_it_decoded(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("recording.ogg");
        std::fs::write(&audio, silent_ogg_opus(75)).unwrap();
        let peaks = dir.path().join("waveform_data").join("recording.peaks");
        let job = running_job(&pool).await;
        let media = crate::media_archive::MediaArchive::disabled();
        let attempt = super::JobAttempt {
            pool: &pool,
            media: &media,
            requester: crate::permissions::Viewer::discord(10),
            id: &job.id,
            token: "token",
        };

        super::decode_and_publish(&attempt, &[(audio, peaks.clone())], "/result")
            .await
            .unwrap();

        assert_eq!(
            read_peaks(&peaks).await.unwrap(),
            Some(vec![FramePeak::SILENT; 75])
        );
        let left: Vec<_> = std::fs::read_dir(peaks.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["recording.peaks"], "no staged file is left behind");
        let status = crate::media_jobs::load_status(&pool, 10, &job.id)
            .await
            .unwrap();
        assert_eq!(
            (status.status.as_str(), status.result_url.as_deref()),
            ("ready", Some("/result"))
        );
    }

    #[sqlx::test(migrations = "../sakiot-db/migrations")]
    async fn a_failed_decode_job_publishes_nothing(pool: sqlx::PgPool) {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("recording.ogg");
        std::fs::write(&audio, b"not an ogg stream").unwrap();
        let peaks = dir.path().join("recording.peaks");
        let job = running_job(&pool).await;
        let media = crate::media_archive::MediaArchive::disabled();
        let attempt = super::JobAttempt {
            pool: &pool,
            media: &media,
            requester: crate::permissions::Viewer::discord(10),
            id: &job.id,
            token: "token",
        };

        assert!(
            super::decode_and_publish(&attempt, &[(audio.clone(), peaks)], "/result")
                .await
                .is_err()
        );
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(left, [audio]);
    }
}

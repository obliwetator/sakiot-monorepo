//! A recording's source file and database state, and whether it has audio
//! to stream yet.

use super::*;

pub(super) async fn source_path(k: &RecordingKey) -> Option<PathBuf> {
    let recording_root = recording_path();
    let padded = k.recording_path(&recording_root);
    if tokio::fs::try_exists(&padded).await.unwrap_or(false) {
        return Some(padded);
    }
    let root = recording_root.trim_end_matches('/');
    let unpadded = PathBuf::from(root)
        .join(format!(
            "{}/{}/{}/{}",
            k.guild_id, k.channel_id, k.year, k.month
        ))
        .join(format!("{}.ogg", k.stem));
    if tokio::fs::try_exists(&unpadded).await.unwrap_or(false) {
        Some(unpadded)
    } else {
        None
    }
}

/// Probe the audio codec of `src`. Returns the lowercase codec name
/// (e.g. "opus", "vorbis"). On any ffprobe failure returns Err.
pub(super) async fn probe_codec(src: &Path) -> Result<String, AppError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(src)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(AppError::IoError)?;
    if !out.status.success() {
        return Err(AppError::FfmpegError("ffprobe failed".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .to_ascii_lowercase())
}

pub(super) async fn db_state(
    pool: &Pool<Postgres>,
    stem: &str,
) -> Result<DbRecordingState, AppError> {
    let row = sqlx::query!(
        "SELECT af.start_ts,
                af.end_ts,
                (
                    af.end_ts IS NULL
                    AND af.reaped IS FALSE
                    AND EXISTS (
                        SELECT 1
                          FROM bot_instances bi
                         WHERE bi.instance_id = af.recording_owner_instance_id
                           AND af.recording_heartbeat_at > now() - interval '120 seconds'
                           AND bi.heartbeat_at > now() - interval '120 seconds'
                           AND bi.state <> 'stopped'
                    )
                ) AS live
           FROM audio_files af
          WHERE af.file_name = $1",
        stem
    )
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(|r| DbRecordingState {
            start_ts: r.start_ts,
            end_ts: r.end_ts,
            live: r.live.unwrap_or(false),
        })
        .unwrap_or(DbRecordingState {
            start_ts: None,
            end_ts: None,
            live: false,
        }))
}

/// The Ogg header pages (OpusHead, OpusTags) and the first audio page.
pub(super) const FIRST_AUDIBLE_PAGES: usize = 3;
/// Enough of a file's start for the header pages and a first audio page of
/// 25 packets at Opus's largest packet size.
const FIRST_PAGES_MAX_BYTES: u64 = 64 * 1024;

/// Complete Ogg pages at the start of `head`, counting up to `want`.
pub(super) fn complete_ogg_pages(head: &[u8], want: usize) -> usize {
    let mut offset = 0;
    let mut pages = 0;
    while pages < want {
        let Some(header) = head.get(offset..offset + 27) else {
            break;
        };
        if &header[..4] != b"OggS" {
            break;
        }
        let segments = usize::from(header[26]);
        let Some(lacing) = head.get(offset + 27..offset + 27 + segments) else {
            break;
        };
        let length = 27 + segments + lacing.iter().map(|&size| usize::from(size)).sum::<usize>();
        if offset + length > head.len() {
            break;
        }
        offset += length;
        pages += 1;
    }
    pages
}

/// Whether a recording's file holds its first audio page yet. The agent
/// creates the file empty and flushes the Ogg headers together with the
/// first audio page, 25 packets (500 ms) later, while the recording's row
/// already says it is live; ffprobe cannot read the file before that.
pub(super) async fn has_first_audio_page(src: &Path) -> std::io::Result<bool> {
    let file = tokio::fs::File::open(src).await?;
    let mut head = Vec::new();
    file.take(FIRST_PAGES_MAX_BYTES)
        .read_to_end(&mut head)
        .await?;
    Ok(complete_ogg_pages(&head, FIRST_AUDIBLE_PAGES) == FIRST_AUDIBLE_PAGES)
}

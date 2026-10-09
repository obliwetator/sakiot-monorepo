//! The HLS playlist on disk: whether a cached build is reusable, finalizing
//! it, and the empty playlist served before the first segment.

use super::*;

async fn playlist_finalized(p: &Path) -> bool {
    matches!(tokio::fs::read_to_string(p).await, Ok(s) if s.contains("#EXT-X-ENDLIST"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HlsCacheAction {
    ReuseFinalized,
    PurgeStale,
    BuildFresh,
}

pub(super) async fn hls_cache_action(playlist: &Path) -> HlsCacheAction {
    if !tokio::fs::try_exists(playlist).await.unwrap_or(false) {
        return HlsCacheAction::BuildFresh;
    }

    if playlist_finalized(playlist).await {
        return HlsCacheAction::ReuseFinalized;
    }

    // A playlist without #EXT-X-ENDLIST means the previous ffmpeg run (live
    // or VOD) never finished — e.g. a crash between spawn and the ENDLIST
    // append. The rebuild cannot reuse it: the VOD command answers prompts
    // with stdin null, so ffmpeg would refuse to overwrite and exit,
    // finalizing the dead playlist. Purge and rebuild from the source.
    HlsCacheAction::PurgeStale
}

pub(super) async fn append_endlist(p: &Path) -> std::io::Result<()> {
    let mut content = tokio::fs::read_to_string(p).await?;
    if content.contains("#EXT-X-ENDLIST") {
        return Ok(());
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("#EXT-X-ENDLIST\n");
    tokio::fs::write(p, content).await
}

/// A live playlist before the first segment: valid, live (no ENDLIST), and
/// empty. hls.js retries an empty live playlist instead of failing.
pub(super) const STARTING_PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:EVENT\n";

/// The answer to a live playlist request while the recording has no audio
/// yet: an empty live playlist, which hls.js reloads.
pub(crate) fn starting_playlist() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .body(STARTING_PLAYLIST)
}

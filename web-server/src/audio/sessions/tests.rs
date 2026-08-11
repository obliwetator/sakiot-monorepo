use super::{
    SessionAccess, composition_progress_percent, session_silence_free_path, timeline_end_ms,
    validate_segment_name,
};

fn access(state: &str) -> SessionAccess {
    SessionAccess {
        session_id: 1,
        guild_id: 2,
        user_id: 3,
        starting_channel_id: 4,
        state: state.to_string(),
        started_at_ms: 1_000,
        ended_at_ms: Some(9_000),
        pause_started_at_ms: Some(5_000),
    }
}

#[test]
fn pending_timeline_stops_at_original_pause() {
    assert_eq!(timeline_end_ms(&access("pending")), 5_000);
    assert_eq!(timeline_end_ms(&access("finalized")), 9_000);
}

#[test]
fn silence_free_cache_identity_uses_the_finalized_session_timestamps() {
    let path = session_silence_free_path(&access("finalized")).unwrap();
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("1-1000-9000.ogg")
    );
    assert!(session_silence_free_path(&access("active")).is_err());
}

#[test]
fn hls_segment_validation_blocks_traversal_and_reserved_routes() {
    assert!(validate_segment_name("seg_00001.m4s").is_ok());
    assert!(validate_segment_name("../secret").is_err());
    assert!(validate_segment_name("playlist.m3u8").is_err());
}

#[test]
fn maps_ffmpeg_output_time_into_composition_progress() {
    assert_eq!(
        composition_progress_percent("out_time_us=5000000", 10_000, 85),
        Some(42)
    );
    assert_eq!(
        composition_progress_percent("out_time_us=10000000", 10_000, 85),
        Some(84)
    );
    assert_eq!(
        composition_progress_percent("progress=end", 10_000, 85),
        None
    );
}

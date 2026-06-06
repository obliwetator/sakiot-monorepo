pub(super) const USER_REJOIN_RESUME_TIMEOUT_MS: u64 = 10 * 60 * 1000;

pub(super) fn silence_frames_for_gap_ms(gap_ms: i64) -> u64 {
    if gap_ms <= 0 {
        0
    } else {
        (gap_ms as u64).div_ceil(20)
    }
}

pub(super) fn paused_timeout_matches(current_token: Option<u64>, timeout_token: u64) -> bool {
    current_token == Some(timeout_token)
}

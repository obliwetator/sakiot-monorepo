pub(super) fn silence_frames_for_gap_ms(gap_ms: i64) -> u64 {
    if gap_ms <= 0 {
        0
    } else {
        (gap_ms as u64).div_ceil(20)
    }
}

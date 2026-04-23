// Re-export the canonical path layout from sakiot-paths. Kept under the old
// names so callers in this crate compile without churn. With trailing slash
// for legacy call sites that concatenated manually.

pub const RECORDING_PATH: &str = "./voice_recordings/";
pub const NO_SILENCE_RECORDING_PATH: &str = "./no_silence_voice_recordings/";
pub const CLIPS_PATH: &str = "./clips/";
pub const NO_SILENCE_PREFIX: &str = sakiot_paths::NO_SILENCE_PREFIX;
pub const WAVEFORM_PATH: &str = "./waveform_data/";

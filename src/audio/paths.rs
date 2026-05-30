pub const NO_SILENCE_PREFIX: &str = sakiot_paths::NO_SILENCE_PREFIX;

fn with_trailing_slash(mut path: String) -> String {
    if !path.ends_with('/') {
        path.push('/');
    }
    path
}

pub fn recording_path() -> String {
    with_trailing_slash(sakiot_paths::DataRoots::from_env().recordings_str())
}

pub fn no_silence_recording_path() -> String {
    with_trailing_slash(sakiot_paths::DataRoots::from_env().no_silence_str())
}

pub fn clips_path() -> String {
    with_trailing_slash(sakiot_paths::DataRoots::from_env().clips_str())
}

pub fn waveform_path() -> String {
    with_trailing_slash(sakiot_paths::DataRoots::from_env().waveforms_str())
}

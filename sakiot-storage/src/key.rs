pub fn recording_object_key(audio_file_id: i64, sha256: &str) -> Option<String> {
    (audio_file_id > 0 && valid_sha256(sha256))
        .then(|| format!("media/v1/recordings/{audio_file_id}/{sha256}.ogg"))
}

pub fn clip_object_key(clip_id: &str, sha256: &str) -> Option<String> {
    (valid_component(clip_id) && valid_sha256(sha256))
        .then(|| format!("media/v1/clips/{clip_id}/{sha256}.ogg"))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_contain_only_source_id_and_digest() {
        let digest = "a".repeat(64);
        assert_eq!(
            recording_object_key(42, &digest),
            Some(format!("media/v1/recordings/42/{digest}.ogg"))
        );
        assert_eq!(
            clip_object_key("opaque-clip", &digest),
            Some(format!("media/v1/clips/opaque-clip/{digest}.ogg"))
        );
        assert_eq!(clip_object_key("guild/clip", &digest), None);
        assert_eq!(clip_object_key("../clip", &digest), None);
        assert_eq!(recording_object_key(-1, &digest), None);
        assert_eq!(recording_object_key(1, &"A".repeat(64)), None);
        assert_eq!(recording_object_key(1, "not-a-digest"), None);
    }
}

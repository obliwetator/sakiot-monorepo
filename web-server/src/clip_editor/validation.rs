use super::*;

pub(super) fn is_valid_clip_id(clip_id: &str) -> bool {
    !clip_id.is_empty()
        && !clip_id.contains("..")
        && !clip_id.contains('/')
        && !clip_id.contains('\\')
        && !clip_id.chars().any(char::is_control)
}

pub(super) fn validate_effects(
    effects: &SegmentEffectsDto,
    index: usize,
    limits: &ComposeLimitsDto,
) -> Result<(), AppError> {
    let advanced = &effects.advanced;
    let all_finite = effects.volume_db.is_finite()
        && effects.pitch_cents.is_finite()
        && effects.rate.is_finite()
        && effects.bass_db.is_finite()
        && effects.mid_db.is_finite()
        && effects.treble_db.is_finite()
        && advanced.tail_seconds.is_finite()
        && advanced.distortion_amount.is_finite()
        && advanced.distortion_wet.is_finite()
        && advanced.delay_seconds.is_finite()
        && advanced.delay_feedback.is_finite()
        && advanced.delay_wet.is_finite()
        && advanced.compressor_threshold_db.is_finite()
        && advanced.compressor_knee_db.is_finite()
        && advanced.compressor_ratio.is_finite()
        && advanced.compressor_attack_seconds.is_finite()
        && advanced.compressor_release_seconds.is_finite()
        && advanced.chorus_frequency_hz.is_finite()
        && advanced.chorus_delay_ms.is_finite()
        && advanced.chorus_depth.is_finite()
        && advanced.chorus_spread_degrees.is_finite()
        && advanced.chorus_feedback.is_finite()
        && advanced.chorus_wet.is_finite()
        && advanced.reverb_decay_seconds.is_finite()
        && advanced.reverb_pre_delay_seconds.is_finite()
        && advanced.reverb_wet.is_finite();
    if !all_finite {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: effect values must be finite"
        )));
    }
    if !(limits.volume_db_min..=limits.volume_db_max).contains(&effects.volume_db) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: volume must be between {} and {} dB",
            limits.volume_db_min, limits.volume_db_max
        )));
    }
    if !(limits.pitch_cents_min..=limits.pitch_cents_max).contains(&effects.pitch_cents) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: pitch must be between {} and {} cents",
            limits.pitch_cents_min, limits.pitch_cents_max
        )));
    }
    if !(limits.rate_min..=limits.rate_max).contains(&effects.rate) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: rate must be between {} and {}",
            limits.rate_min, limits.rate_max
        )));
    }
    if !(limits.bass_db_min..=limits.bass_db_max).contains(&effects.bass_db)
        || !(limits.mid_db_min..=limits.mid_db_max).contains(&effects.mid_db)
        || !(limits.treble_db_min..=limits.treble_db_max).contains(&effects.treble_db)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: EQ gains must be between {} and {} dB",
            limits.bass_db_min, limits.bass_db_max
        )));
    }
    if !(0.0..=sakiot_dsp::MAX_EFFECT_TAIL_SECONDS).contains(&advanced.tail_seconds) {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: effect tail must be between 0 and {} seconds",
            sakiot_dsp::MAX_EFFECT_TAIL_SECONDS
        )));
    }
    if !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.distortion_amount)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.distortion_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: distortion amount and wet must be between 0 and 1"
        )));
    }
    if !(0.0..=DELAY_MAX_SECONDS).contains(&advanced.delay_seconds)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.delay_feedback)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.delay_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: delay parameters are outside the supported ranges"
        )));
    }
    if !(-100.0..=0.0).contains(&advanced.compressor_threshold_db)
        || !(0.0..=40.0).contains(&advanced.compressor_knee_db)
        || !(1.0..=20.0).contains(&advanced.compressor_ratio)
        || !(0.0..=1.0).contains(&advanced.compressor_attack_seconds)
        || !(0.0..=1.0).contains(&advanced.compressor_release_seconds)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: compressor parameters are outside the supported ranges"
        )));
    }
    if !(0.0..=20.0).contains(&advanced.chorus_frequency_hz)
        || !(0.0..=100.0).contains(&advanced.chorus_delay_ms)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_depth)
        || !(0.0..=360.0).contains(&advanced.chorus_spread_degrees)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_feedback)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.chorus_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: chorus parameters are outside the supported ranges"
        )));
    }
    if !(0.001..=30.0).contains(&advanced.reverb_decay_seconds)
        || !(0.0..=5.0).contains(&advanced.reverb_pre_delay_seconds)
        || !(NORMALIZED_MIN..=NORMALIZED_MAX).contains(&advanced.reverb_wet)
    {
        return Err(AppError::BadRequest(format!(
            "Segment {index}: reverb parameters are outside the supported ranges"
        )));
    }
    Ok(())
}

pub(super) fn advanced_effects_active(effects: &SegmentEffectsDto) -> bool {
    effects.advanced.tail_seconds > 0.0
        || effects.advanced.distortion_wet > 0.0
        || effects.advanced.delay_wet > 0.0
        || effects.advanced.compressor_enabled
        || effects.advanced.chorus_enabled
        || effects.advanced.reverb_enabled
}

pub(super) fn validate_edit(body: &ComposeClipBody) -> Result<(), AppError> {
    if body.segments.is_empty() || body.segments.len() > MAX_SEGMENTS {
        return Err(AppError::BadRequest(format!(
            "Composition must contain between 1 and {MAX_SEGMENTS} segments"
        )));
    }
    if let Some(overwrite_clip_id) = &body.overwrite_clip_id
        && !is_valid_clip_id(overwrite_clip_id)
    {
        return Err(AppError::BadRequest("Invalid overwrite clip id".into()));
    }
    let limits = body.limits.unwrap_or_default().validated()?;
    if !body.master_volume_db.is_finite()
        || !(VOLUME_MIN..=VOLUME_MAX).contains(&body.master_volume_db)
    {
        return Err(AppError::BadRequest(format!(
            "Master volume must be between {VOLUME_MIN} and {VOLUME_MAX} dB"
        )));
    }
    if body.muted_tracks.len() > MAX_TRACKS as usize + 1 {
        return Err(AppError::BadRequest(format!(
            "muted_tracks cannot contain more than {} tracks",
            MAX_TRACKS + 1
        )));
    }
    for (index, segment) in body.segments.iter().enumerate() {
        if segment.source != "clip" {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: only \"clip\" sources are supported"
            )));
        }
        if !is_valid_clip_id(&segment.source_id) {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: invalid source clip id"
            )));
        }
        if !(0..=MAX_TRACKS).contains(&segment.track) {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: track must be between 0 and {MAX_TRACKS}"
            )));
        }
        if let Some(group) = &segment.merge_group
            && !is_valid_clip_id(group)
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: invalid merge group id"
            )));
        }
        if !segment.source_in.is_finite()
            || !segment.source_out.is_finite()
            || segment.source_in < 0.0
            || segment.source_out - segment.source_in < MIN_SEGMENT_SECONDS
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: source range is invalid"
            )));
        }
        if !segment.timeline_start.is_finite() || segment.timeline_start < 0.0 {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: timeline_start must be non-negative"
            )));
        }
        validate_effects(&segment.effects, index, &limits)?;
        if segment.source_out - segment.source_in > MAX_SHARED_DSP_SEGMENT_SECONDS
            && advanced_effects_active(&segment.effects)
        {
            return Err(AppError::BadRequest(format!(
                "Segment {index}: shared tail, distortion, delay, compressor, chorus, and reverb currently support source windows up to {MAX_SHARED_DSP_SEGMENT_SECONDS} seconds"
            )));
        }
    }
    let total_seconds = body
        .segments
        .iter()
        .map(|segment| {
            f64::from(segment.timeline_start)
                + f64::from(segment.source_out - segment.source_in)
                    / f64::from(segment.effects.rate)
                + f64::from(segment.effects.advanced.tail_seconds)
        })
        .fold(0.0, f64::max);
    if total_seconds > MAX_TOTAL_SECONDS {
        return Err(AppError::BadRequest(format!(
            "Composition is longer than {MAX_TOTAL_SECONDS} seconds"
        )));
    }
    Ok(())
}

pub(super) fn validate_composition(
    body: ComposeClipBody,
) -> Result<ValidatedComposition, AppError> {
    validate_edit(&body)?;
    Ok(ValidatedComposition(body))
}

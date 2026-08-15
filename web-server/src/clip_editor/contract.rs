use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComposeClipBody {
    pub name: Option<String>,
    pub master_volume_db: f32,
    /// Per-track mute state, indexed by track number. Missing values are
    /// treated as unmuted so older compositions remain compatible.
    #[serde(default)]
    pub muted_tracks: Vec<bool>,
    pub segments: Vec<ComposeSegment>,
    /// Id of an existing composed clip to replace with this export. Only
    /// composed clips (`original_file_name = 'compose'`) can be overwritten;
    /// the clip keeps its id and owner, and its name when `name` is absent.
    #[serde(default)]
    pub overwrite_clip_id: Option<String>,
    /// Per-user adjustable slider bounds for the six effects the editor lets
    /// users widen (volume, pitch, rate, and the three EQ bands). Defaults to
    /// the doubled editor ranges when absent; each pair is additionally
    /// bounded by absolute safety caps so renders cannot overflow f32 to
    /// INF/NaN or exhaust memory.
    #[serde(default)]
    pub limits: Option<ComposeLimitsDto>,
}

/// Slider bounds for the adjustable effects, matching the frontend's per-user
/// limit settings. Missing fields fall back to the doubled default ranges.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct ComposeLimitsDto {
    pub volume_db_min: f32,
    pub volume_db_max: f32,
    pub pitch_cents_min: f32,
    pub pitch_cents_max: f32,
    pub rate_min: f32,
    pub rate_max: f32,
    pub bass_db_min: f32,
    pub bass_db_max: f32,
    pub mid_db_min: f32,
    pub mid_db_max: f32,
    pub treble_db_min: f32,
    pub treble_db_max: f32,
}

impl Default for ComposeLimitsDto {
    fn default() -> Self {
        Self {
            volume_db_min: -80.0,
            volume_db_max: 24.0,
            pitch_cents_min: -2_400.0,
            pitch_cents_max: 2_400.0,
            rate_min: 0.25,
            rate_max: 4.0,
            bass_db_min: -24.0,
            bass_db_max: 24.0,
            mid_db_min: -24.0,
            mid_db_max: 24.0,
            treble_db_min: -24.0,
            treble_db_max: 24.0,
        }
    }
}

fn validate_limit_pair(
    name: &str,
    minimum: f32,
    maximum: f32,
    safety_min: f32,
    safety_max: f32,
) -> Result<(), AppError> {
    if !minimum.is_finite() || !maximum.is_finite() {
        return Err(AppError::BadRequest(format!(
            "{name} limits must be finite"
        )));
    }
    if minimum >= maximum {
        return Err(AppError::BadRequest(format!(
            "{name} minimum must be below the maximum"
        )));
    }
    if minimum < safety_min || maximum > safety_max {
        return Err(AppError::BadRequest(format!(
            "{name} limits are outside the supported {safety_min}..{safety_max} range"
        )));
    }
    Ok(())
}

impl ComposeLimitsDto {
    pub(super) fn validated(self) -> Result<Self, AppError> {
        validate_limit_pair(
            "volume",
            self.volume_db_min,
            self.volume_db_max,
            -LIMIT_GAIN_MAX_ABS_DB,
            LIMIT_GAIN_MAX_ABS_DB,
        )?;
        validate_limit_pair(
            "pitch",
            self.pitch_cents_min,
            self.pitch_cents_max,
            -LIMIT_PITCH_MAX_ABS_CENTS,
            LIMIT_PITCH_MAX_ABS_CENTS,
        )?;
        validate_limit_pair(
            "rate",
            self.rate_min,
            self.rate_max,
            LIMIT_RATE_MIN,
            LIMIT_RATE_MAX,
        )?;
        validate_limit_pair(
            "bass",
            self.bass_db_min,
            self.bass_db_max,
            -LIMIT_GAIN_MAX_ABS_DB,
            LIMIT_GAIN_MAX_ABS_DB,
        )?;
        validate_limit_pair(
            "mid",
            self.mid_db_min,
            self.mid_db_max,
            -LIMIT_GAIN_MAX_ABS_DB,
            LIMIT_GAIN_MAX_ABS_DB,
        )?;
        validate_limit_pair(
            "treble",
            self.treble_db_min,
            self.treble_db_max,
            -LIMIT_GAIN_MAX_ABS_DB,
            LIMIT_GAIN_MAX_ABS_DB,
        )?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComposeSegment {
    pub source: String,
    pub source_id: String,
    pub source_in: f32,
    pub source_out: f32,
    pub timeline_start: f32,
    pub track: i32,
    pub effects: SegmentEffectsDto,
    /// Id of the merged unit this segment belongs to, when the clip editor
    /// merged several snapped segments into one element. Segments sharing an
    /// id re-import as one unit. Purely an editing hint: the render ignores
    /// it. Optional so compositions from before the field existed still
    /// deserialize.
    #[serde(default)]
    pub merge_group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SegmentEffectsDto {
    pub volume_db: f32,
    pub pitch_cents: f32,
    pub rate: f32,
    pub bass_db: f32,
    /// 1 kHz peaking EQ gain. Defaults to zero for saved compositions from
    /// before the parity-matched mid control existed.
    #[serde(default)]
    pub mid_db: f32,
    pub treble_db: f32,
    /// Plays the trimmed content backwards. Defaults to false so requests and
    /// stored compositions from before the flag existed still deserialize.
    #[serde(default)]
    pub reverse: bool,
    /// Stateful and modulation effects added by the shared DSP integration.
    /// The entire group defaults to bypass-compatible DSP defaults for older
    /// saved compositions.
    #[serde(default)]
    pub advanced: AdvancedSegmentEffectsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct AdvancedSegmentEffectsDto {
    /// Fixed silence appended after reverse/pitch/rate and processed through
    /// the shared effect chain, making effect ring-out part of the timeline.
    pub tail_seconds: f32,
    pub distortion_amount: f32,
    pub distortion_wet: f32,
    pub delay_seconds: f32,
    pub delay_feedback: f32,
    pub delay_wet: f32,
    pub compressor_enabled: bool,
    pub compressor_threshold_db: f32,
    pub compressor_knee_db: f32,
    pub compressor_ratio: f32,
    pub compressor_attack_seconds: f32,
    pub compressor_release_seconds: f32,
    pub chorus_enabled: bool,
    pub chorus_frequency_hz: f32,
    pub chorus_delay_ms: f32,
    pub chorus_depth: f32,
    pub chorus_spread_degrees: f32,
    pub chorus_feedback: f32,
    pub chorus_wet: f32,
    pub reverb_enabled: bool,
    pub reverb_decay_seconds: f32,
    pub reverb_pre_delay_seconds: f32,
    pub reverb_wet: f32,
    pub reverb_seed: u32,
}

impl Default for AdvancedSegmentEffectsDto {
    fn default() -> Self {
        let effects = sakiot_dsp::SegmentEffects::default();
        Self {
            tail_seconds: effects.tail_seconds,
            distortion_amount: effects.distortion_amount,
            distortion_wet: effects.distortion_wet,
            delay_seconds: effects.delay_seconds,
            delay_feedback: effects.delay_feedback,
            delay_wet: effects.delay_wet,
            compressor_enabled: effects.compressor_enabled,
            compressor_threshold_db: effects.compressor_threshold_db,
            compressor_knee_db: effects.compressor_knee_db,
            compressor_ratio: effects.compressor_ratio,
            compressor_attack_seconds: effects.compressor_attack_seconds,
            compressor_release_seconds: effects.compressor_release_seconds,
            chorus_enabled: effects.chorus_enabled,
            chorus_frequency_hz: effects.chorus_frequency_hz,
            chorus_delay_ms: effects.chorus_delay_ms,
            chorus_depth: effects.chorus_depth,
            chorus_spread_degrees: effects.chorus_spread_degrees,
            chorus_feedback: effects.chorus_feedback,
            chorus_wet: effects.chorus_wet,
            reverb_enabled: effects.reverb_enabled,
            reverb_decay_seconds: effects.reverb_decay_seconds,
            reverb_pre_delay_seconds: effects.reverb_pre_delay_seconds,
            reverb_wet: effects.reverb_wet,
            reverb_seed: effects.reverb_seed,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ComposeClipAccepted {
    pub status: &'static str,
    pub progress: i16,
    pub id: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ComposeClipStatus {
    pub status: String,
    pub progress: i16,
}

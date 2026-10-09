//! Shared audio processing for the Sakiot clip editor.
//!
//! The core deliberately has no browser or server dependencies. The optional
//! `wasm` feature only adds a thin `wasm-bindgen` boundary around the same
//! [`SegmentProcessor`] used by native callers.

mod biquad;
mod chorus;
mod compressor;
mod delay;
mod distortion;
mod fft;
mod incremental;
mod offline;
mod reverb;

pub use incremental::{IncrementalRenderer, StreamError};

pub use offline::{render_clip_interleaved, reverse_interleaved_frames};

use biquad::*;
use chorus::*;
use compressor::{Compressor, CompressorParameters};
use delay::*;
use distortion::*;
use reverb::{Reverb, ReverbParameters};

use std::error::Error;
use std::f64::consts::PI;
use std::fmt::{Display, Formatter};

const BASS_FREQUENCY_HZ: f64 = 250.0;
const MID_FREQUENCY_HZ: f64 = 1_000.0;
const TREBLE_FREQUENCY_HZ: f64 = 3_000.0;
const MID_Q: f64 = 1.0;
// Tone.Distortion constructs WaveShaper with `length: 4096`, but its amount
// setter calls `setMap` without forwarding that length. Tone 15.1.22 therefore
// replaces the curve with WaveShaper.setMap's 1,024-sample default.
const DISTORTION_CURVE_LENGTH: usize = 1_024;
const WEB_AUDIO_RENDER_QUANTUM_FRAMES: usize = 128;
const WEB_AUDIO_DELAY_FRACTION_STEPS: f64 = 256.0;
pub const MAX_EFFECT_TAIL_SECONDS: f32 = 30.0;
/// Short output correction ramp used when live parameters change. Offline
/// renders start with their final configuration and therefore never enter it.
const PARAMETER_SMOOTHING_SECONDS: f64 = 0.005;

/// The complete effect parameter boundary used by the current clip editor.
///
/// Streaming effects are applied by [`SegmentProcessor`]. Length-changing
/// `pitch_cents` and `rate`, plus source-order `reverse`, are applied by
/// [`render_clip_interleaved`] before that streaming chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentEffects {
    pub volume_db: f32,
    pub pitch_cents: f32,
    pub rate: f32,
    /// Silence appended after reverse and pitch/rate, but before the streaming
    /// chain, so delay and reverb can ring out for this exact duration.
    pub tail_seconds: f32,
    pub bass_db: f32,
    pub mid_db: f32,
    pub treble_db: f32,
    /// Tone-compatible distortion amount. Tone documents a nominal 0..=1
    /// range, though its implementation accepts any finite value.
    pub distortion_amount: f32,
    /// Equal-power dry/wet mix, matching Tone.Effect's normalized range.
    pub distortion_wet: f32,
    /// Feedback delay time in seconds.
    pub delay_seconds: f32,
    /// Amount of the delayed output returned to the delay input.
    pub delay_feedback: f32,
    /// Equal-power dry/wet mix for the feedback delay.
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
    pub reverse: bool,
}

impl Default for SegmentEffects {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            pitch_cents: 0.0,
            rate: 1.0,
            tail_seconds: 0.0,
            bass_db: 0.0,
            mid_db: 0.0,
            treble_db: 0.0,
            distortion_amount: 0.4,
            distortion_wet: 0.0,
            delay_seconds: 0.25,
            delay_feedback: 0.125,
            delay_wet: 0.0,
            compressor_enabled: false,
            compressor_threshold_db: -24.0,
            compressor_knee_db: 30.0,
            compressor_ratio: 12.0,
            compressor_attack_seconds: 0.003,
            compressor_release_seconds: 0.25,
            chorus_enabled: false,
            chorus_frequency_hz: 1.5,
            chorus_delay_ms: 3.5,
            chorus_depth: 0.7,
            chorus_spread_degrees: 180.0,
            chorus_feedback: 0.0,
            chorus_wet: 0.5,
            reverb_enabled: false,
            reverb_decay_seconds: 1.5,
            reverb_pre_delay_seconds: 0.01,
            reverb_wet: 1.0,
            reverb_seed: 0x5341_4b49,
            reverse: false,
        }
    }
}

impl SegmentEffects {
    fn is_finite(self) -> bool {
        self.volume_db.is_finite()
            && self.pitch_cents.is_finite()
            && self.rate.is_finite()
            && self.tail_seconds.is_finite()
            && self.bass_db.is_finite()
            && self.mid_db.is_finite()
            && self.treble_db.is_finite()
            && self.distortion_amount.is_finite()
            && self.distortion_wet.is_finite()
            && self.delay_seconds.is_finite()
            && self.delay_feedback.is_finite()
            && self.delay_wet.is_finite()
            && self.compressor_threshold_db.is_finite()
            && self.compressor_knee_db.is_finite()
            && self.compressor_ratio.is_finite()
            && self.compressor_attack_seconds.is_finite()
            && self.compressor_release_seconds.is_finite()
            && self.chorus_frequency_hz.is_finite()
            && self.chorus_delay_ms.is_finite()
            && self.chorus_depth.is_finite()
            && self.chorus_spread_degrees.is_finite()
            && self.chorus_feedback.is_finite()
            && self.chorus_wet.is_finite()
            && self.reverb_decay_seconds.is_finite()
            && self.reverb_pre_delay_seconds.is_finite()
            && self.reverb_wet.is_finite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectCoverage {
    pub volume: bool,
    pub equalizer: bool,
    pub distortion: bool,
    pub feedback_delay: bool,
    pub compressor: bool,
    pub chorus: bool,
    pub reverb: bool,
    pub pitch: bool,
    pub rate: bool,
    pub reverse: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DspError {
    StreamFinished,
    StreamLengthMismatch,
    ReverseRequiresOrderedInput,
    InvalidSampleRate,
    InvalidChannelCount,
    InvalidEffects,
    MisalignedInterleavedBuffer,
    ChannelOutOfRange,
    InterleavedFramesRequired,
    UnsupportedFfmpegEffects,
}

impl Display for DspError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::StreamFinished => "segment has already been finalized",
            Self::StreamLengthMismatch => "PCM frame count differs from the declared source length",
            Self::ReverseRequiresOrderedInput => {
                "reverse requires supplying source frames in reverse order"
            }
            Self::InvalidSampleRate => "sample rate must be finite and greater than 6 kHz",
            Self::InvalidChannelCount => "channel count must be between 1 and 32",
            Self::InvalidEffects => "effect parameters are outside the supported editor ranges",
            Self::MisalignedInterleavedBuffer => {
                "interleaved buffer length must be divisible by the channel count"
            }
            Self::ChannelOutOfRange => "channel index is outside the processor channel count",
            Self::InterleavedFramesRequired => {
                "stereo-linked effects require interleaved frame processing"
            }
            Self::UnsupportedFfmpegEffects => {
                "the FFmpeg bridge only supports volume and equalizer effects"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for DspError {}

/// Stateful, block-size-independent processor for one timeline segment.
///
/// Processing order is part of the DSP contract and cannot be changed by the
/// order in which parameters are set: volume, bass/mid/treble EQ, distortion,
/// feedback delay, chorus, compressor, then reverb. The offline renderer runs
/// reverse and the combined pitch/rate transform before this streaming chain.
#[derive(Debug)]
pub struct SegmentProcessor {
    sample_rate: f64,
    channels: usize,
    effects: SegmentEffects,
    volume_gain: f32,
    bass: Biquad,
    mid: Biquad,
    treble: Biquad,
    distortion: Distortion,
    delay: FeedbackDelay,
    chorus: Chorus,
    compressor: Compressor,
    reverb: Reverb,
    output_smoothers: Vec<OutputSmoother>,
    smoothing_frames: usize,
}

/// A parameter update may change several nonlinear/stateful processors at
/// once, so interpolating individual coefficients is not universally safe.
/// Instead, preserve output continuity and decay that correction to the new
/// chain over a deterministic number of samples.
#[derive(Debug, Clone, Copy, Default)]
struct OutputSmoother {
    last_output: f32,
    correction: f32,
    remaining_frames: usize,
    has_output: bool,
    pending: bool,
}

impl SegmentProcessor {
    pub fn new(
        sample_rate: f64,
        channels: usize,
        effects: SegmentEffects,
    ) -> Result<Self, DspError> {
        if !sample_rate.is_finite() || sample_rate <= 6_000.0 {
            return Err(DspError::InvalidSampleRate);
        }
        if !(1..=32).contains(&channels) {
            return Err(DspError::InvalidChannelCount);
        }
        validate_effects(effects)?;
        Ok(Self::new_validated(sample_rate, channels, effects))
    }

    fn new_validated(sample_rate: f64, channels: usize, effects: SegmentEffects) -> Self {
        let mut processor = Self {
            sample_rate,
            channels,
            effects,
            volume_gain: 1.0,
            bass: Biquad::new(channels),
            mid: Biquad::new(channels),
            treble: Biquad::new(channels),
            distortion: Distortion::new(effects.distortion_amount, effects.distortion_wet),
            delay: FeedbackDelay::new(
                sample_rate,
                channels,
                effects.delay_seconds,
                effects.delay_feedback,
                effects.delay_wet,
            ),
            chorus: Chorus::new(sample_rate, channels, effects),
            compressor: Compressor::new(sample_rate, channels, compressor_parameters(effects)),
            reverb: Reverb::new(sample_rate, channels, reverb_parameters(effects)),
            output_smoothers: vec![OutputSmoother::default(); channels],
            smoothing_frames: (sample_rate * PARAMETER_SMOOTHING_SECONDS).round().max(1.0) as usize,
        };
        processor.update_coefficients();
        processor
    }

    pub const fn coverage() -> EffectCoverage {
        EffectCoverage {
            volume: true,
            equalizer: true,
            distortion: true,
            feedback_delay: true,
            compressor: true,
            chorus: true,
            reverb: true,
            pitch: true,
            rate: true,
            reverse: true,
        }
    }

    pub fn effects(&self) -> SegmentEffects {
        self.effects
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Change parameters without clearing filter history, matching live
    /// parameter changes in a Web Audio graph.
    pub fn set_effects(&mut self, effects: SegmentEffects) -> Result<(), DspError> {
        validate_effects(effects)?;
        if self.effects == effects {
            return Ok(());
        }
        let compressor_toggled = self.effects.compressor_enabled != effects.compressor_enabled;
        let chorus_toggled = self.effects.chorus_enabled != effects.chorus_enabled;
        let reverb_toggled = self.effects.reverb_enabled != effects.reverb_enabled;
        self.effects = effects;
        self.update_coefficients();
        if compressor_toggled {
            self.compressor.reset();
        }
        if chorus_toggled {
            self.chorus.reset();
        }
        if reverb_toggled {
            self.reverb.reset();
        }
        for smoother in &mut self.output_smoothers {
            smoother.pending = smoother.has_output;
        }
        Ok(())
    }

    /// Clear delay elements when playback seeks, restarts, or reuses a node.
    pub fn reset(&mut self) {
        self.bass.reset();
        self.mid.reset();
        self.treble.reset();
        self.delay.reset();
        self.chorus.reset();
        self.compressor.reset();
        self.reverb.reset();
        self.output_smoothers.fill(OutputSmoother::default());
    }

    /// Process interleaved PCM in place.
    pub fn process_interleaved(&mut self, samples: &mut [f32]) -> Result<(), DspError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(DspError::MisalignedInterleavedBuffer);
        }
        for frame in samples.chunks_exact_mut(self.channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = self.process_sample(channel, *sample);
            }
            if self.effects.chorus_enabled {
                self.chorus.process_frame(frame);
            }
            if self.effects.compressor_enabled {
                self.compressor.process_frame(frame);
            }
            if self.effects.reverb_enabled {
                self.reverb.process_frame(frame);
            }
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = self.smooth_output(channel, *sample);
            }
        }
        Ok(())
    }

    /// Process one planar Web Audio channel in place. All channels in a block
    /// should be processed before processing the next block.
    pub fn process_channel(&mut self, channel: usize, samples: &mut [f32]) -> Result<(), DspError> {
        if channel >= self.channels {
            return Err(DspError::ChannelOutOfRange);
        }
        if (self.effects.compressor_enabled
            || self.effects.chorus_enabled
            || self.effects.reverb_enabled)
            && self.channels > 1
        {
            return Err(DspError::InterleavedFramesRequired);
        }
        for sample in samples {
            *sample = self.process_sample(channel, *sample);
            if self.effects.chorus_enabled {
                self.chorus.process_frame(std::slice::from_mut(sample));
            }
            if self.effects.compressor_enabled {
                self.compressor.process_frame(std::slice::from_mut(sample));
            }
            if self.effects.reverb_enabled {
                self.reverb.process_frame(std::slice::from_mut(sample));
            }
            *sample = self.smooth_output(channel, *sample);
        }
        Ok(())
    }

    fn smooth_output(&mut self, channel: usize, output: f32) -> f32 {
        let smoother = &mut self.output_smoothers[channel];
        if smoother.pending {
            smoother.correction = smoother.last_output - output;
            smoother.remaining_frames = self.smoothing_frames;
            smoother.pending = false;
        }
        let smoothed = if smoother.remaining_frames > 0 {
            let fraction = smoother.remaining_frames as f32 / self.smoothing_frames as f32;
            smoother.remaining_frames -= 1;
            output + smoother.correction * fraction
        } else {
            output
        };
        smoother.last_output = smoothed;
        smoother.has_output = true;
        smoothed
    }

    fn process_sample(&mut self, channel: usize, input: f32) -> f32 {
        let sample = input * self.volume_gain;
        let sample = self.bass.process(channel, sample);
        let sample = self.mid.process(channel, sample);
        let sample = self.treble.process(channel, sample);
        let sample = self.distortion.process(sample);
        self.delay.process(channel, sample)
    }

    fn update_coefficients(&mut self) {
        self.volume_gain = db_to_gain(self.effects.volume_db);
        self.bass.set_coefficients(low_shelf(
            self.sample_rate,
            BASS_FREQUENCY_HZ,
            f64::from(self.effects.bass_db),
        ));
        self.mid.set_coefficients(peaking(
            self.sample_rate,
            MID_FREQUENCY_HZ,
            MID_Q,
            f64::from(self.effects.mid_db),
        ));
        self.treble.set_coefficients(high_shelf(
            self.sample_rate,
            TREBLE_FREQUENCY_HZ,
            f64::from(self.effects.treble_db),
        ));
        self.distortion
            .update(self.effects.distortion_amount, self.effects.distortion_wet);
        self.delay.update(
            self.sample_rate,
            self.effects.delay_seconds,
            self.effects.delay_feedback,
            self.effects.delay_wet,
        );
        self.compressor.update(compressor_parameters(self.effects));
        self.chorus.update(self.effects);
        self.reverb.update(reverb_parameters(self.effects));
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    libm::powf(10.0, db / 20.0)
}

fn equal_power_gains(wet: f32) -> (f32, f32) {
    if wet <= 0.0 {
        (1.0, 0.0)
    } else if wet >= 1.0 {
        (0.0, 1.0)
    } else {
        let angle = f64::from(wet) * PI / 2.0;
        (libm::cos(angle) as f32, libm::sin(angle) as f32)
    }
}

/// Build an FFmpeg filter chain from the canonical coefficients used by this
/// processor. Only the FFmpeg parity test uses it, to check the processor
/// against FFmpeg's own filters; the server renders with the processor itself.
pub fn ffmpeg_filter_chain(sample_rate: f64, effects: SegmentEffects) -> Result<String, DspError> {
    if !sample_rate.is_finite() || sample_rate <= 6_000.0 {
        return Err(DspError::InvalidSampleRate);
    }
    validate_effects(effects)?;
    if effects.distortion_wet > 0.0
        || effects.delay_wet > 0.0
        || effects.compressor_enabled
        || effects.chorus_enabled
        || effects.reverb_enabled
    {
        return Err(DspError::UnsupportedFfmpegEffects);
    }
    let coefficients = [
        low_shelf(sample_rate, BASS_FREQUENCY_HZ, f64::from(effects.bass_db)),
        peaking(
            sample_rate,
            MID_FREQUENCY_HZ,
            MID_Q,
            f64::from(effects.mid_db),
        ),
        high_shelf(
            sample_rate,
            TREBLE_FREQUENCY_HZ,
            f64::from(effects.treble_db),
        ),
    ];
    let mut filters = vec![format!("volume={:.9}", db_to_gain(effects.volume_db))];
    for coefficient in coefficients {
        filters.push(format!(
            "biquad=a0=1:a1={:.9}:a2={:.9}:b0={:.9}:b1={:.9}:b2={:.9}:a=tdii:r=f32",
            coefficient.a1, coefficient.a2, coefficient.b0, coefficient.b1, coefficient.b2,
        ));
    }
    Ok(filters.join(","))
}

fn validate_effects(effects: SegmentEffects) -> Result<(), DspError> {
    if !effects.is_finite()
        || !(-4_800.0..=4_800.0).contains(&effects.pitch_cents)
        || !(0.1..=10.0).contains(&effects.rate)
        || !(0.0..=MAX_EFFECT_TAIL_SECONDS).contains(&effects.tail_seconds)
        || effects.delay_seconds < 0.0
        || !(0.0..=1.0).contains(&effects.distortion_wet)
        || !(0.0..=1.0).contains(&effects.delay_feedback)
        || !(0.0..=1.0).contains(&effects.delay_wet)
        || !(-100.0..=0.0).contains(&effects.compressor_threshold_db)
        || !(0.0..=40.0).contains(&effects.compressor_knee_db)
        || !(1.0..=20.0).contains(&effects.compressor_ratio)
        || !(0.0..=1.0).contains(&effects.compressor_attack_seconds)
        || !(0.0..=1.0).contains(&effects.compressor_release_seconds)
        || !(0.0..=20.0).contains(&effects.chorus_frequency_hz)
        || !(0.0..=100.0).contains(&effects.chorus_delay_ms)
        || !(0.0..=1.0).contains(&effects.chorus_depth)
        || !(0.0..=360.0).contains(&effects.chorus_spread_degrees)
        || !(0.0..=1.0).contains(&effects.chorus_feedback)
        || !(0.0..=1.0).contains(&effects.chorus_wet)
        || !(0.001..=30.0).contains(&effects.reverb_decay_seconds)
        || !(0.0..=5.0).contains(&effects.reverb_pre_delay_seconds)
        || !(0.0..=1.0).contains(&effects.reverb_wet)
    {
        return Err(DspError::InvalidEffects);
    }
    Ok(())
}

fn compressor_parameters(effects: SegmentEffects) -> CompressorParameters {
    CompressorParameters {
        threshold_db: effects.compressor_threshold_db,
        knee_db: effects.compressor_knee_db,
        ratio: effects.compressor_ratio,
        attack_seconds: effects.compressor_attack_seconds,
        release_seconds: effects.compressor_release_seconds,
    }
}

fn reverb_parameters(effects: SegmentEffects) -> ReverbParameters {
    ReverbParameters {
        enabled: effects.reverb_enabled,
        decay_seconds: effects.reverb_decay_seconds,
        pre_delay_seconds: effects.reverb_pre_delay_seconds,
        wet: effects.reverb_wet,
        seed: effects.reverb_seed,
    }
}

#[cfg(feature = "wasm")]
mod wasm;

#[cfg(test)]
mod tests;

//! Shared audio processing prototype for the Sakiot clip editor.
//!
//! The core deliberately has no browser or server dependencies. The optional
//! `wasm` feature only adds a thin `wasm-bindgen` boundary around the same
//! [`SegmentProcessor`] used by native callers.

mod compressor;
mod fft;
mod offline;
mod reverb;

pub use offline::{render_clip_interleaved, reverse_interleaved_frames};

use compressor::{Compressor, CompressorParameters};
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

#[derive(Debug, Clone, Copy)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FilterState {
    z1: f32,
    z2: f32,
}

#[derive(Debug)]
struct Biquad {
    coefficients: Coefficients,
    states: Vec<FilterState>,
}

impl Biquad {
    fn new(channels: usize) -> Self {
        Self {
            coefficients: Coefficients::IDENTITY,
            states: vec![FilterState::default(); channels],
        }
    }

    fn set_coefficients(&mut self, coefficients: Coefficients) {
        self.coefficients = coefficients;
    }

    fn process(&mut self, channel: usize, input: f32) -> f32 {
        let state = &mut self.states[channel];
        let coefficients = self.coefficients;
        // Transposed direct form II: two state values per channel and no
        // dependence on the caller's block size.
        let output = coefficients.b0 * input + state.z1;
        state.z1 = coefficients.b1 * input - coefficients.a1 * output + state.z2;
        state.z2 = coefficients.b2 * input - coefficients.a2 * output;
        output
    }

    fn reset(&mut self) {
        self.states.fill(FilterState::default());
    }
}

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

/// Tone 15's Distortion effect ultimately stores its mapping in a 1,024-sample Web Audio
/// WaveShaper curve. Keeping the sampled curve (instead of evaluating the
/// analytic expression directly) lets native and WASM follow the browser's
/// actual interpolation behavior.
#[derive(Debug)]
struct Distortion {
    curve: Vec<f32>,
    dry_gain: f32,
    wet_gain: f32,
}

impl Distortion {
    fn new(amount: f32, wet: f32) -> Self {
        let mut distortion = Self {
            curve: vec![0.0; DISTORTION_CURVE_LENGTH],
            dry_gain: 1.0,
            wet_gain: 0.0,
        };
        distortion.update(amount, wet);
        distortion
    }

    fn update(&mut self, amount: f32, wet: f32) {
        let k = f64::from(amount) * 100.0;
        let degrees = PI / 180.0;
        let denominator = (DISTORTION_CURVE_LENGTH - 1) as f64;
        for (index, value) in self.curve.iter_mut().enumerate() {
            let input = (index as f64 / denominator) * 2.0 - 1.0;
            *value = if input.abs() < 0.001 {
                0.0
            } else {
                (((3.0 + k) * input * 20.0 * degrees) / (PI + k * input.abs())) as f32
            };
        }
        (self.dry_gain, self.wet_gain) = equal_power_gains(wet);
    }

    fn process(&self, input: f32) -> f32 {
        if self.wet_gain == 0.0 {
            return input;
        }
        let clamped = input.clamp(-1.0, 1.0);
        let position = (f64::from(clamped) + 1.0) * 0.5 * (DISTORTION_CURVE_LENGTH - 1) as f64;
        let lower = (position.floor() as usize).min(DISTORTION_CURVE_LENGTH - 1);
        let upper = (lower + 1).min(DISTORTION_CURVE_LENGTH - 1);
        let fraction = (position - lower as f64) as f32;
        let shaped = self.curve[lower] + (self.curve[upper] - self.curve[lower]) * fraction;
        input * self.dry_gain + shaped * self.wet_gain
    }
}

#[derive(Debug)]
struct DelayChannel {
    buffer: Vec<f32>,
    write_index: usize,
    feedback_buffer: [f32; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
    feedback_index: usize,
}

#[derive(Debug)]
struct FeedbackDelay {
    channels: Vec<DelayChannel>,
    delay_samples: f64,
    feedback: f32,
    dry_gain: f32,
    wet_gain: f32,
}

impl FeedbackDelay {
    fn new(sample_rate: f64, channels: usize, delay_seconds: f32, feedback: f32, wet: f32) -> Self {
        let delay_samples = web_audio_delay_samples(sample_rate, delay_seconds);
        let buffer_length = delay_samples.ceil() as usize + 2;
        let mut delay = Self {
            channels: (0..channels)
                .map(|_| DelayChannel {
                    buffer: vec![0.0; buffer_length],
                    write_index: 0,
                    feedback_buffer: [0.0; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
                    feedback_index: 0,
                })
                .collect(),
            delay_samples,
            feedback,
            dry_gain: 1.0,
            wet_gain: 0.0,
        };
        delay.update(sample_rate, delay_seconds, feedback, wet);
        delay
    }

    fn update(&mut self, sample_rate: f64, delay_seconds: f32, feedback: f32, wet: f32) {
        let delay_samples = web_audio_delay_samples(sample_rate, delay_seconds);
        let required_length = delay_samples.ceil() as usize + 2;
        if self.channels[0].buffer.len() != required_length {
            for channel in &mut self.channels {
                channel.buffer = vec![0.0; required_length];
                channel.write_index = 0;
                channel.feedback_buffer.fill(0.0);
                channel.feedback_index = 0;
            }
        }
        self.delay_samples = delay_samples;
        self.feedback = feedback;
        (self.dry_gain, self.wet_gain) = equal_power_gains(wet);
    }

    fn process(&mut self, channel: usize, input: f32) -> f32 {
        if self.wet_gain == 0.0 {
            return input;
        }
        let state = &mut self.channels[channel];
        let length = state.buffer.len();
        let read_position =
            (state.write_index as f64 - self.delay_samples).rem_euclid(length as f64);
        let lower = read_position.floor() as usize;
        let upper = (lower + 1) % length;
        let fraction = (read_position - lower as f64) as f32;
        let delayed = state.buffer[lower] + (state.buffer[upper] - state.buffer[lower]) * fraction;
        // Web Audio breaks feedback graph cycles at a 128-frame render quantum.
        // Tone.FeedbackDelay therefore returns each feedback echo 128 frames
        // later than a conventional sample-level delay line. Mirror that
        // observable timing while the existing Tone graph is the parity target.
        let feedback_return = state.feedback_buffer[state.feedback_index];
        state.buffer[state.write_index] = input + feedback_return * self.feedback;
        state.write_index = (state.write_index + 1) % length;
        state.feedback_buffer[state.feedback_index] = delayed;
        state.feedback_index = (state.feedback_index + 1) % WEB_AUDIO_RENDER_QUANTUM_FRAMES;
        input * self.dry_gain + delayed * self.wet_gain
    }

    fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.buffer.fill(0.0);
            channel.write_index = 0;
            channel.feedback_buffer.fill(0.0);
            channel.feedback_index = 0;
        }
    }
}

#[derive(Debug)]
struct ChorusChannel {
    delay_buffer: Vec<f32>,
    write_index: usize,
    feedback_buffer: [f32; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
    feedback_index: usize,
}

#[derive(Debug)]
struct Chorus {
    sample_rate: f64,
    channels: Vec<ChorusChannel>,
    frequency_hz: f32,
    center_delay_seconds: f32,
    depth: f32,
    spread_degrees: f32,
    feedback: f32,
    dry_gain: f32,
    wet_gain: f32,
    frame_index: u64,
}

impl Chorus {
    fn new(sample_rate: f64, channel_count: usize, effects: SegmentEffects) -> Self {
        let buffer_length = chorus_buffer_length(sample_rate, effects);
        let (dry_gain, wet_gain) = equal_power_gains(effects.chorus_wet);
        Self {
            sample_rate,
            channels: (0..channel_count)
                .map(|_| ChorusChannel {
                    delay_buffer: vec![0.0; buffer_length],
                    write_index: 0,
                    feedback_buffer: [0.0; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
                    feedback_index: 0,
                })
                .collect(),
            frequency_hz: effects.chorus_frequency_hz,
            center_delay_seconds: effects.chorus_delay_ms / 1_000.0,
            depth: effects.chorus_depth,
            spread_degrees: effects.chorus_spread_degrees,
            feedback: effects.chorus_feedback,
            dry_gain,
            wet_gain,
            frame_index: 0,
        }
    }

    fn update(&mut self, effects: SegmentEffects) {
        let required_length = chorus_buffer_length(self.sample_rate, effects);
        if self.channels[0].delay_buffer.len() != required_length {
            for channel in &mut self.channels {
                channel.delay_buffer = vec![0.0; required_length];
                channel.write_index = 0;
                channel.feedback_buffer.fill(0.0);
                channel.feedback_index = 0;
            }
        }
        self.frequency_hz = effects.chorus_frequency_hz;
        self.center_delay_seconds = effects.chorus_delay_ms / 1_000.0;
        self.depth = effects.chorus_depth;
        self.spread_degrees = effects.chorus_spread_degrees;
        self.feedback = effects.chorus_feedback;
        (self.dry_gain, self.wet_gain) = equal_power_gains(effects.chorus_wet);
    }

    fn process_frame(&mut self, frame: &mut [f32]) {
        let time = self.frame_index as f64 / self.sample_rate;
        for (channel_index, sample) in frame.iter_mut().enumerate() {
            let phase_degrees = if channel_index % 2 == 0 {
                90.0 - f64::from(self.spread_degrees) / 2.0
            } else {
                90.0 + f64::from(self.spread_degrees) / 2.0
            };
            let phase = phase_degrees * PI / 180.0;
            let lfo = (2.0 * PI * f64::from(self.frequency_hz) * time - phase).sin();
            let delay_seconds =
                f64::from(self.center_delay_seconds) * (1.0 + f64::from(self.depth) * lfo);
            let delay_samples =
                ((self.sample_rate * delay_seconds * WEB_AUDIO_DELAY_FRACTION_STEPS).round()
                    / WEB_AUDIO_DELAY_FRACTION_STEPS)
                    .max(1.0);

            let state = &mut self.channels[channel_index];
            let length = state.delay_buffer.len();
            let read_position =
                (state.write_index as f64 - delay_samples).rem_euclid(length as f64);
            let lower = read_position.floor() as usize;
            let upper = (lower + 1) % length;
            let fraction = (read_position - lower as f64) as f32;
            let delayed = state.delay_buffer[lower]
                + (state.delay_buffer[upper] - state.delay_buffer[lower]) * fraction;
            let feedback_return = state.feedback_buffer[state.feedback_index];
            let input = *sample;
            state.delay_buffer[state.write_index] = input + feedback_return * self.feedback;
            state.write_index = (state.write_index + 1) % length;
            state.feedback_buffer[state.feedback_index] = delayed;
            state.feedback_index = (state.feedback_index + 1) % WEB_AUDIO_RENDER_QUANTUM_FRAMES;
            *sample = input * self.dry_gain + delayed * self.wet_gain;
        }
        self.frame_index += 1;
    }

    fn reset(&mut self) {
        self.frame_index = 0;
        for channel in &mut self.channels {
            channel.delay_buffer.fill(0.0);
            channel.write_index = 0;
            channel.feedback_buffer.fill(0.0);
            channel.feedback_index = 0;
        }
    }
}

fn chorus_buffer_length(sample_rate: f64, effects: SegmentEffects) -> usize {
    let maximum_delay_seconds =
        f64::from(effects.chorus_delay_ms * (1.0 + effects.chorus_depth)) / 1_000.0;
    ((sample_rate * maximum_delay_seconds).ceil() as usize + 2).max(3)
}

fn web_audio_delay_samples(sample_rate: f64, delay_seconds: f32) -> f64 {
    // Chromium's DelayNode uses 8 fractional bits for its interpolated read
    // position. Matching that 1/256-frame quantization removes a small but
    // measurable difference for non-integer delay times.
    ((sample_rate * f64::from(delay_seconds) * WEB_AUDIO_DELAY_FRACTION_STEPS).round()
        / WEB_AUDIO_DELAY_FRACTION_STEPS)
        .max(1.0)
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
    10.0_f32.powf(db / 20.0)
}

fn equal_power_gains(wet: f32) -> (f32, f32) {
    if wet <= 0.0 {
        (1.0, 0.0)
    } else if wet >= 1.0 {
        (0.0, 1.0)
    } else {
        let angle = f64::from(wet) * PI / 2.0;
        (angle.cos() as f32, angle.sin() as f32)
    }
}

/// Build an FFmpeg filter chain from the canonical coefficients used by this
/// processor. This adapter is for prototype comparisons and a future native
/// integration fallback; the existing server continues using its named EQ
/// filters while the experiment is isolated.
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

fn peaking(sample_rate: f64, frequency: f64, q: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = 10.0_f64.powf(gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let alpha = omega.sin() / (2.0 * q);
    Coefficients::normalized(
        1.0 + alpha * a,
        -2.0 * omega.cos(),
        1.0 - alpha * a,
        1.0 + alpha / a,
        -2.0 * omega.cos(),
        1.0 - alpha / a,
    )
}

fn low_shelf(sample_rate: f64, frequency: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = 10.0_f64.powf(gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * 2.0_f64.sqrt() / 2.0;
    let beta = 2.0 * a.sqrt() * alpha;
    Coefficients::normalized(
        a * ((a + 1.0) - (a - 1.0) * cosine + beta),
        2.0 * a * ((a - 1.0) - (a + 1.0) * cosine),
        a * ((a + 1.0) - (a - 1.0) * cosine - beta),
        (a + 1.0) + (a - 1.0) * cosine + beta,
        -2.0 * ((a - 1.0) + (a + 1.0) * cosine),
        (a + 1.0) + (a - 1.0) * cosine - beta,
    )
}

fn high_shelf(sample_rate: f64, frequency: f64, gain_db: f64) -> Coefficients {
    if gain_db == 0.0 {
        return Coefficients::IDENTITY;
    }
    let a = 10.0_f64.powf(gain_db / 40.0);
    let omega = 2.0 * PI * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * 2.0_f64.sqrt() / 2.0;
    let beta = 2.0 * a.sqrt() * alpha;
    Coefficients::normalized(
        a * ((a + 1.0) + (a - 1.0) * cosine + beta),
        -2.0 * a * ((a - 1.0) + (a + 1.0) * cosine),
        a * ((a + 1.0) + (a - 1.0) * cosine - beta),
        (a + 1.0) - (a - 1.0) * cosine + beta,
        2.0 * ((a - 1.0) - (a + 1.0) * cosine),
        (a + 1.0) - (a - 1.0) * cosine - beta,
    )
}

#[cfg(feature = "wasm")]
mod wasm {
    use super::{SegmentEffects, SegmentProcessor};
    use js_sys::Reflect;
    use wasm_bindgen::prelude::*;

    const EFFECT_CONFIG_VERSION: f64 = 2.0;

    fn field(object: &JsValue, name: &str) -> Option<JsValue> {
        Reflect::get(object, &JsValue::from_str(name)).ok()
    }

    fn number(object: &JsValue, name: &str) -> Option<f32> {
        let value = field(object, name)?.as_f64()?;
        let value = value as f32;
        value.is_finite().then_some(value)
    }

    fn boolean(object: &JsValue, name: &str) -> Option<bool> {
        field(object, name)?.as_bool()
    }

    fn effect_config(config: &JsValue) -> Option<SegmentEffects> {
        if field(config, "version")?.as_f64()? != EFFECT_CONFIG_VERSION {
            return None;
        }
        let effects = field(config, "effects")?;
        if !effects.is_object() {
            return None;
        }
        let reverb_seed = field(&effects, "reverbSeed")?.as_f64()?;
        if !reverb_seed.is_finite()
            || reverb_seed.fract() != 0.0
            || !(0.0..=u32::MAX as f64).contains(&reverb_seed)
        {
            return None;
        }
        Some(SegmentEffects {
            volume_db: number(&effects, "volumeDb")?,
            pitch_cents: number(&effects, "pitchCents")?,
            rate: number(&effects, "rate")?,
            tail_seconds: number(&effects, "tailSeconds")?,
            bass_db: number(&effects, "bassDb")?,
            mid_db: number(&effects, "midDb")?,
            treble_db: number(&effects, "trebleDb")?,
            distortion_amount: number(&effects, "distortionAmount")?,
            distortion_wet: number(&effects, "distortionWet")?,
            delay_seconds: number(&effects, "delaySeconds")?,
            delay_feedback: number(&effects, "delayFeedback")?,
            delay_wet: number(&effects, "delayWet")?,
            compressor_enabled: boolean(&effects, "compressorEnabled")?,
            compressor_threshold_db: number(&effects, "compressorThresholdDb")?,
            compressor_knee_db: number(&effects, "compressorKneeDb")?,
            compressor_ratio: number(&effects, "compressorRatio")?,
            compressor_attack_seconds: number(&effects, "compressorAttackSeconds")?,
            compressor_release_seconds: number(&effects, "compressorReleaseSeconds")?,
            chorus_enabled: boolean(&effects, "chorusEnabled")?,
            chorus_frequency_hz: number(&effects, "chorusFrequencyHz")?,
            chorus_delay_ms: number(&effects, "chorusDelayMs")?,
            chorus_depth: number(&effects, "chorusDepth")?,
            chorus_spread_degrees: number(&effects, "chorusSpreadDegrees")?,
            chorus_feedback: number(&effects, "chorusFeedback")?,
            chorus_wet: number(&effects, "chorusWet")?,
            reverb_enabled: boolean(&effects, "reverbEnabled")?,
            reverb_decay_seconds: number(&effects, "reverbDecaySeconds")?,
            reverb_pre_delay_seconds: number(&effects, "reverbPreDelaySeconds")?,
            reverb_wet: number(&effects, "reverbWet")?,
            reverb_seed: reverb_seed as u32,
            reverse: boolean(&effects, "reverse")?,
        })
    }

    /// Copy-based prototype boundary. A production AudioWorklet may use the
    /// WASM linear memory directly after profiling this simpler version.
    #[wasm_bindgen]
    pub struct WasmSegmentProcessor {
        inner: SegmentProcessor,
    }

    #[wasm_bindgen]
    impl WasmSegmentProcessor {
        #[wasm_bindgen(constructor)]
        pub fn new(sample_rate: f64, channels: usize) -> WasmSegmentProcessor {
            let sample_rate = if sample_rate.is_finite() && sample_rate > 6_000.0 {
                sample_rate
            } else {
                48_000.0
            };
            Self {
                inner: SegmentProcessor::new_validated(
                    sample_rate,
                    channels.clamp(1, 32),
                    SegmentEffects::default(),
                ),
            }
        }

        /// Apply a complete, versioned JavaScript configuration object. The
        /// boundary intentionally rejects missing fields or unknown versions
        /// instead of silently mixing schemas.
        pub fn set_effect_config(&mut self, config: &JsValue) -> bool {
            effect_config(config)
                .and_then(|effects| self.inner.set_effects(effects).ok())
                .is_some()
        }

        pub fn process_interleaved(&mut self, samples: &mut [f32]) -> bool {
            self.inner.process_interleaved(samples).is_ok()
        }

        /// Offline clip path for length-changing rate/pitch transforms and
        /// frame-order reverse. The returned buffer is newly allocated.
        pub fn render_clip_interleaved(&self, samples: &[f32]) -> Vec<f32> {
            super::render_clip_interleaved(
                samples,
                self.inner.sample_rate,
                self.inner.channels,
                self.inner.effects,
            )
            .unwrap_or_default()
        }

        pub fn reset(&mut self) {
            self.inner.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DspError, PARAMETER_SMOOTHING_SECONDS, SegmentEffects, SegmentProcessor, db_to_gain,
        render_clip_interleaved,
    };
    use std::f32::consts::TAU;

    const SAMPLE_RATE: f64 = 48_000.0;

    fn signal(frames: usize, channels: usize) -> Vec<f32> {
        let mut output = Vec::with_capacity(frames * channels);
        let mut noise = 0x1234_5678_u32;
        for frame in 0..frames {
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            let random = (noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
            for channel in 0..channels {
                let frequency = 173.0 + channel as f32 * 619.0;
                let sine = (TAU * frequency * frame as f32 / SAMPLE_RATE as f32).sin();
                output.push(sine * 0.2 + random * 0.05);
            }
        }
        output
    }

    fn configured() -> SegmentEffects {
        SegmentEffects {
            volume_db: -3.0,
            bass_db: 5.0,
            mid_db: -4.0,
            treble_db: 2.5,
            ..SegmentEffects::default()
        }
    }

    #[test]
    fn bypass_is_bit_exact() {
        let mut samples = signal(1_024, 2);
        let original = samples.clone();
        let mut processor = SegmentProcessor::new(SAMPLE_RATE, 2, SegmentEffects::default())
            .expect("valid processor");
        processor
            .process_interleaved(&mut samples)
            .expect("aligned buffer");
        assert_eq!(samples, original);
    }

    #[test]
    fn volume_uses_decibel_amplitude_conversion() {
        let effects = SegmentEffects {
            volume_db: -6.0,
            ..SegmentEffects::default()
        };
        let mut processor =
            SegmentProcessor::new(SAMPLE_RATE, 1, effects).expect("valid processor");
        let mut samples = [1.0, -0.5];
        processor
            .process_interleaved(&mut samples)
            .expect("aligned buffer");
        let gain = db_to_gain(-6.0);
        assert!((samples[0] - gain).abs() < 1e-7);
        assert!((samples[1] + gain * 0.5).abs() < 1e-7);
    }

    #[test]
    fn distortion_bypasses_at_zero_wet_and_clamps_the_curve_input() {
        let input = [-2.0, -1.0, -0.25, 0.0, 0.25, 1.0, 2.0];
        let mut bypassed = input;
        let mut bypass = SegmentProcessor::new(SAMPLE_RATE, 1, SegmentEffects::default())
            .expect("valid processor");
        bypass
            .process_interleaved(&mut bypassed)
            .expect("aligned buffer");
        assert_eq!(bypassed, input);

        let mut shaped = input;
        let effects = SegmentEffects {
            distortion_amount: 0.7,
            distortion_wet: 1.0,
            ..SegmentEffects::default()
        };
        SegmentProcessor::new(SAMPLE_RATE, 1, effects)
            .expect("valid processor")
            .process_interleaved(&mut shaped)
            .expect("aligned buffer");
        assert_eq!(shaped[3], 0.0);
        assert_eq!(shaped[0], shaped[1]);
        assert_eq!(shaped[5], shaped[6]);
        assert!(shaped[0] >= -1.0 && shaped[6] <= 1.0);
    }

    #[test]
    fn feedback_delay_places_sample_accurate_echoes() {
        let effects = SegmentEffects {
            delay_seconds: 0.125,
            delay_feedback: 0.5,
            delay_wet: 1.0,
            ..SegmentEffects::default()
        };
        let mut samples = vec![0.0; 3_257];
        samples[0] = 1.0;
        SegmentProcessor::new(8_000.0, 1, effects)
            .expect("valid processor")
            .process_interleaved(&mut samples)
            .expect("aligned buffer");
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[1_000], 1.0);
        assert_eq!(samples[2_128], 0.5);
        assert_eq!(samples[3_256], 0.25);
        assert_eq!(samples.iter().filter(|sample| **sample != 0.0).count(), 3);
    }

    #[test]
    fn compressor_is_block_independent_and_requires_linked_stereo_frames() {
        let effects = SegmentEffects {
            compressor_enabled: true,
            ..SegmentEffects::default()
        };
        let input = signal(4_097, 2);
        let mut whole = input.clone();
        let mut chunked = input.clone();
        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        let mut chunked_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        whole_processor
            .process_interleaved(&mut whole)
            .expect("aligned buffer");
        for chunk in chunked.chunks_mut(74) {
            chunked_processor
                .process_interleaved(chunk)
                .expect("37 stereo frames");
        }
        assert_eq!(whole, chunked);

        assert_eq!(
            whole_processor.process_channel(0, &mut [0.0; 32]),
            Err(DspError::InterleavedFramesRequired)
        );
    }

    #[test]
    fn compressor_has_the_web_audio_six_millisecond_lookahead() {
        let effects = SegmentEffects {
            compressor_enabled: true,
            ..SegmentEffects::default()
        };
        let mut samples = vec![0.0; 600 * 2];
        samples[0] = 0.5;
        SegmentProcessor::new(SAMPLE_RATE, 2, effects)
            .expect("valid processor")
            .process_interleaved(&mut samples)
            .expect("aligned buffer");
        assert!(samples[..288 * 2].iter().all(|sample| *sample == 0.0));
        assert_ne!(samples[288 * 2], 0.0);
        assert_eq!(samples[288 * 2 + 1], 0.0);
    }

    #[test]
    fn chorus_is_stereo_and_block_independent() {
        let effects = SegmentEffects {
            chorus_enabled: true,
            chorus_wet: 1.0,
            ..SegmentEffects::default()
        };
        let input = signal(4_097, 2);
        let mut whole = input.clone();
        let mut chunked = input.clone();
        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        let mut chunked_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        whole_processor
            .process_interleaved(&mut whole)
            .expect("aligned buffer");
        for chunk in chunked.chunks_mut(74) {
            chunked_processor
                .process_interleaved(chunk)
                .expect("37 stereo frames");
        }
        assert_eq!(whole, chunked);
        assert!(whole.chunks_exact(2).any(|frame| frame[0] != frame[1]));
    }

    #[test]
    fn seeded_reverb_is_deterministic_stereo_and_block_independent() {
        let effects = SegmentEffects {
            reverb_enabled: true,
            reverb_decay_seconds: 0.05,
            reverb_pre_delay_seconds: 0.002,
            reverb_wet: 1.0,
            reverb_seed: 0x1234_5678,
            ..SegmentEffects::default()
        };
        let input = signal(2_049, 2);
        let mut whole = input.clone();
        let mut chunked = input;
        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        let mut chunked_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        whole_processor
            .process_interleaved(&mut whole)
            .expect("aligned buffer");
        for chunk in chunked.chunks_mut(74) {
            chunked_processor
                .process_interleaved(chunk)
                .expect("37 stereo frames");
        }
        assert_eq!(whole, chunked);
        assert!(whole.chunks_exact(2).any(|frame| frame[0] != frame[1]));

        whole_processor.reset();
        let mut repeated = signal(2_049, 2);
        whole_processor
            .process_interleaved(&mut repeated)
            .expect("aligned buffer");
        assert_eq!(whole, repeated);
    }

    #[test]
    fn complete_stateful_chain_is_independent_of_audio_worklet_blocks() {
        let effects = SegmentEffects {
            volume_db: -3.0,
            bass_db: 5.0,
            mid_db: -4.0,
            treble_db: 2.5,
            distortion_amount: 0.7,
            distortion_wet: 1.0,
            delay_seconds: 0.125,
            delay_feedback: 0.4,
            delay_wet: 0.35,
            compressor_enabled: true,
            chorus_enabled: true,
            ..SegmentEffects::default()
        };
        let input = signal(8_192, 2);
        let mut whole = input.clone();
        let mut worklet_blocks = input.clone();
        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        let mut block_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, effects).expect("valid processor");
        whole_processor
            .process_interleaved(&mut whole)
            .expect("aligned buffer");
        for block in worklet_blocks.chunks_mut(128 * 2) {
            block_processor
                .process_interleaved(block)
                .expect("128 stereo frames");
        }
        assert_eq!(whole, worklet_blocks);
    }

    #[test]
    fn processing_is_independent_of_interleaved_block_size() {
        let input = signal(4_097, 2);
        let mut whole = input.clone();
        let mut chunked = input.clone();
        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, configured()).expect("valid processor");
        let mut chunked_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, configured()).expect("valid processor");
        whole_processor
            .process_interleaved(&mut whole)
            .expect("aligned buffer");
        for chunk in chunked.chunks_mut(74) {
            chunked_processor
                .process_interleaved(chunk)
                .expect("37 stereo frames");
        }
        assert_eq!(whole, chunked);
    }

    #[test]
    fn planar_and_interleaved_processing_match() {
        let input = signal(2_048, 2);
        let mut interleaved = input.clone();
        let mut left: Vec<f32> = input.iter().step_by(2).copied().collect();
        let mut right: Vec<f32> = input.iter().skip(1).step_by(2).copied().collect();
        let mut interleaved_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, configured()).expect("valid processor");
        let mut planar_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, configured()).expect("valid processor");
        interleaved_processor
            .process_interleaved(&mut interleaved)
            .expect("aligned buffer");
        planar_processor
            .process_channel(0, &mut left)
            .expect("left channel");
        planar_processor
            .process_channel(1, &mut right)
            .expect("right channel");
        for (frame, expected) in interleaved.chunks_exact(2).zip(left.iter().zip(&right)) {
            assert_eq!(frame, [*expected.0, *expected.1]);
        }
    }

    #[test]
    fn reset_reproduces_the_initial_response() {
        let input = signal(512, 1);
        let mut first = input.clone();
        let mut second = input.clone();
        let mut processor =
            SegmentProcessor::new(SAMPLE_RATE, 1, configured()).expect("valid processor");
        processor
            .process_interleaved(&mut first)
            .expect("aligned buffer");
        processor.reset();
        processor
            .process_interleaved(&mut second)
            .expect("aligned buffer");
        assert_eq!(first, second);
    }

    #[test]
    fn live_parameter_changes_preserve_output_continuity_and_settle() {
        let mut processor =
            SegmentProcessor::new(SAMPLE_RATE, 1, SegmentEffects::default()).expect("valid");
        let mut before = [1.0; 16];
        processor
            .process_interleaved(&mut before)
            .expect("initial signal");
        processor
            .set_effects(SegmentEffects {
                volume_db: -60.0,
                ..SegmentEffects::default()
            })
            .expect("valid update");

        let smoothing_frames = (SAMPLE_RATE * PARAMETER_SMOOTHING_SECONDS) as usize;
        let mut after = vec![1.0; smoothing_frames + 2];
        processor
            .process_interleaved(&mut after)
            .expect("updated signal");

        assert_eq!(after[0], before[before.len() - 1]);
        assert!(after.windows(2).all(|pair| pair[1] <= pair[0]));
        assert!((after[smoothing_frames] - db_to_gain(-60.0)).abs() < 1e-7);
    }

    #[test]
    fn parameter_smoothing_is_independent_of_processing_block_size() {
        let input = signal(1_024, 2);
        let (first, second) = input.split_at(128 * 2);
        let updated = SegmentEffects {
            volume_db: -18.0,
            distortion_amount: 0.8,
            distortion_wet: 0.7,
            ..SegmentEffects::default()
        };

        let mut whole_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, SegmentEffects::default()).expect("valid");
        let mut chunked_processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, SegmentEffects::default()).expect("valid");
        let mut whole_first = first.to_vec();
        let mut chunked_first = first.to_vec();
        whole_processor
            .process_interleaved(&mut whole_first)
            .expect("whole prefix");
        chunked_processor
            .process_interleaved(&mut chunked_first)
            .expect("chunked prefix");
        whole_processor.set_effects(updated).expect("whole update");
        chunked_processor
            .set_effects(updated)
            .expect("chunked update");

        let mut whole_second = second.to_vec();
        let mut chunked_second = second.to_vec();
        whole_processor
            .process_interleaved(&mut whole_second)
            .expect("whole suffix");
        for chunk in chunked_second.chunks_mut(74) {
            chunked_processor
                .process_interleaved(chunk)
                .expect("37 stereo frames");
        }
        assert_eq!(whole_first, chunked_first);
        assert_eq!(whole_second, chunked_second);
    }

    #[test]
    fn rejects_invalid_boundaries() {
        assert!(matches!(
            SegmentProcessor::new(SAMPLE_RATE, 0, SegmentEffects::default()),
            Err(DspError::InvalidChannelCount)
        ));
        let mut processor =
            SegmentProcessor::new(SAMPLE_RATE, 2, SegmentEffects::default()).expect("valid");
        assert_eq!(
            processor.process_interleaved(&mut [0.0]),
            Err(DspError::MisalignedInterleavedBuffer)
        );
        assert_eq!(
            processor.set_effects(SegmentEffects {
                rate: 0.0,
                ..SegmentEffects::default()
            }),
            Err(DspError::InvalidEffects)
        );
    }

    #[test]
    fn accepts_the_widened_editor_limits_and_rejects_the_safety_caps() {
        let widened = SegmentEffects {
            volume_db: 24.0,
            pitch_cents: 2_400.0,
            rate: 0.25,
            bass_db: -24.0,
            mid_db: 24.0,
            treble_db: -24.0,
            ..SegmentEffects::default()
        };
        SegmentProcessor::new(SAMPLE_RATE, 2, widened).expect("widened limits valid");
        let rejected = SegmentProcessor::new(
            SAMPLE_RATE,
            2,
            SegmentEffects {
                pitch_cents: 4_801.0,
                ..SegmentEffects::default()
            },
        )
        .expect_err("pitch beyond the safety cap is invalid");
        assert_eq!(rejected, DspError::InvalidEffects);
        let rejected = SegmentProcessor::new(
            SAMPLE_RATE,
            2,
            SegmentEffects {
                rate: 10.1,
                ..SegmentEffects::default()
            },
        )
        .expect_err("rate beyond the safety cap is invalid");
        assert_eq!(rejected, DspError::InvalidEffects);
    }

    #[test]
    fn offline_default_render_is_bit_exact() {
        let input = signal(4_097, 2);
        let output = render_clip_interleaved(&input, SAMPLE_RATE, 2, SegmentEffects::default())
            .expect("valid offline render");
        assert_eq!(output, input);
    }

    #[test]
    fn offline_tail_appends_silence_before_streaming_effects() {
        let sample_rate = 8_000.0;
        let mut input = vec![0.0; 100];
        input[0] = 1.0;
        let output = render_clip_interleaved(
            &input,
            sample_rate,
            1,
            SegmentEffects {
                tail_seconds: 0.05,
                delay_seconds: 0.0125,
                delay_feedback: 0.0,
                delay_wet: 1.0,
                ..SegmentEffects::default()
            },
        )
        .expect("tail render");

        assert_eq!(output.len(), 500);
        assert_eq!(output[100], 1.0);
        assert_eq!(output.iter().filter(|sample| **sample != 0.0).count(), 1);
    }

    #[test]
    fn canonical_rate_and_pitch_have_independent_duration_and_frequency() {
        let frames = 12_000;
        let input: Vec<f32> = (0..frames)
            .map(|frame| (TAU * 440.0 * frame as f32 / SAMPLE_RATE as f32).sin() * 0.25)
            .collect();
        let rate_only = render_clip_interleaved(
            &input,
            SAMPLE_RATE,
            1,
            SegmentEffects {
                rate: 1.5,
                ..SegmentEffects::default()
            },
        )
        .expect("rate render");
        assert_eq!(rate_only.len(), 8_000);
        assert!((positive_crossing_frequency(&rate_only, SAMPLE_RATE as f32) - 440.0).abs() < 8.0);

        let pitched = render_clip_interleaved(
            &input,
            SAMPLE_RATE,
            1,
            SegmentEffects {
                pitch_cents: 1_200.0,
                ..SegmentEffects::default()
            },
        )
        .expect("pitch render");
        assert_eq!(pitched.len(), frames);
        assert!((positive_crossing_frequency(&pitched, SAMPLE_RATE as f32) - 880.0).abs() < 12.0);
    }

    #[test]
    fn reverse_flips_complete_frames_before_rate_and_pitch() {
        let input = vec![1.0, 10.0, 2.0, 20.0, 3.0, 30.0, 4.0, 40.0];
        let reversed = render_clip_interleaved(
            &input,
            SAMPLE_RATE,
            2,
            SegmentEffects {
                reverse: true,
                ..SegmentEffects::default()
            },
        )
        .expect("reverse render");
        assert_eq!(reversed, [4.0, 40.0, 3.0, 30.0, 2.0, 20.0, 1.0, 10.0]);

        let input = signal(4_097, 2);
        // Build the expected reversed source without swapping channel samples.
        let source_reversed: Vec<f32> = input
            .chunks_exact(2)
            .rev()
            .flat_map(|frame| frame.iter().copied())
            .collect();
        let transformed_reverse = render_clip_interleaved(
            &input,
            SAMPLE_RATE,
            2,
            SegmentEffects {
                pitch_cents: 700.0,
                rate: 1.25,
                reverse: true,
                ..SegmentEffects::default()
            },
        )
        .expect("combined reverse render");
        let transformed_expected = render_clip_interleaved(
            &source_reversed,
            SAMPLE_RATE,
            2,
            SegmentEffects {
                pitch_cents: 700.0,
                rate: 1.25,
                ..SegmentEffects::default()
            },
        )
        .expect("pre-reversed render");
        assert_eq!(transformed_reverse, transformed_expected);
    }

    fn positive_crossing_frequency(samples: &[f32], sample_rate: f32) -> f32 {
        let trim = samples.len().min(2_048);
        let body = &samples[trim..samples.len().saturating_sub(trim)];
        let crossings = body
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f32 * sample_rate / body.len() as f32
    }
}

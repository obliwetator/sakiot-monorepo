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

/// Block preprocessing; reverse is supplied by the caller's source traversal.
#[wasm_bindgen]
pub struct WasmIncrementalRenderer {
    inner: super::IncrementalRenderer,
}

#[wasm_bindgen]
impl WasmIncrementalRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(
        sample_rate: f64,
        channels: usize,
        frames: usize,
        config: &JsValue,
    ) -> Result<WasmIncrementalRenderer, JsValue> {
        let effects = effect_config(config)
            .ok_or_else(|| JsValue::from_str("Invalid effect configuration"))?;
        let inner = super::IncrementalRenderer::new(sample_rate, channels, frames, effects)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { inner })
    }
    pub fn output_frames(&self) -> usize {
        self.inner.output_frames()
    }
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, JsValue> {
        if samples.len() > 8192 {
            return Err(JsValue::from_str("DSP input block exceeds 8192 samples"));
        }
        let mut output = Vec::new();
        self.inner
            .push(samples, |block: &[f32]| {
                output.extend_from_slice(block);
                Ok::<_, std::convert::Infallible>(())
            })
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Vec<f32>, JsValue> {
        let mut output = Vec::new();
        self.inner
            .finish(|block: &[f32]| {
                output.extend_from_slice(block);
                Ok::<_, std::convert::Infallible>(())
            })
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(output)
    }
}

/// Copy-based boundary: samples are copied into and out of WASM memory.
/// Working in the linear memory directly is an option if profiling calls
/// for it.
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

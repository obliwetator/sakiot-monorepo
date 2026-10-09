use std::f64::consts::PI;

use crate::{DISTORTION_CURVE_LENGTH, equal_power_gains};

/// Tone 15's Distortion effect ultimately stores its mapping in a 1,024-sample Web Audio
/// WaveShaper curve. Keeping the sampled curve (instead of evaluating the
/// analytic expression directly) lets native and WASM follow the browser's
/// actual interpolation behavior.
#[derive(Debug)]
pub(crate) struct Distortion {
    curve: Vec<f32>,
    dry_gain: f32,
    wet_gain: f32,
}

impl Distortion {
    pub(crate) fn new(amount: f32, wet: f32) -> Self {
        let mut distortion = Self {
            curve: vec![0.0; DISTORTION_CURVE_LENGTH],
            dry_gain: 1.0,
            wet_gain: 0.0,
        };
        distortion.update(amount, wet);
        distortion
    }

    pub(crate) fn update(&mut self, amount: f32, wet: f32) {
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

    pub(crate) fn process(&self, input: f32) -> f32 {
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

// This implementation follows Chromium/Blink's Web Audio dynamics compressor
// signal model. Original implementation copyright (C) 2011 Google Inc., under
// Chromium's BSD-style license:
// https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/renderer/platform/audio/dynamics_compressor.cc

use std::f32::consts::FRAC_PI_2;

const PRE_DELAY_SECONDS: f32 = 0.006;
const MAX_PRE_DELAY_FRAMES: usize = 1_024;
const PRE_DELAY_MASK: usize = MAX_PRE_DELAY_FRAMES - 1;
const DIVISION_FRAMES: usize = 32;
const SAT_RELEASE_SECONDS: f32 = 0.0025;

const RELEASE_ZONE_1: f32 = 0.09;
const RELEASE_ZONE_2: f32 = 0.16;
const RELEASE_ZONE_3: f32 = 0.42;
const RELEASE_ZONE_4: f32 = 0.98;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompressorParameters {
    pub threshold_db: f32,
    pub knee_db: f32,
    pub ratio: f32,
    pub attack_seconds: f32,
    pub release_seconds: f32,
}

#[derive(Debug)]
pub(crate) struct Compressor {
    sample_rate: f32,
    channels: usize,
    parameters: CompressorParameters,
    pre_delay_buffers: Vec<Vec<f32>>,
    pre_delay_read_index: usize,
    pre_delay_write_index: usize,
    detector_average: f32,
    compressor_gain: f32,
    max_attack_compression_diff_db: f32,
    division_remaining: usize,
    envelope_rate: f32,
    linear_post_gain: f32,
    scaled_desired_gain: f32,
    slope: f32,
    linear_threshold: f32,
    threshold_db: f32,
    knee_db: f32,
    knee_threshold: f32,
    knee_threshold_db: f32,
    knee_output_db: f32,
    knee: f32,
}

impl Compressor {
    pub(crate) fn new(sample_rate: f64, channels: usize, parameters: CompressorParameters) -> Self {
        let mut compressor = Self {
            sample_rate: sample_rate as f32,
            channels,
            parameters,
            pre_delay_buffers: vec![vec![0.0; MAX_PRE_DELAY_FRAMES]; channels],
            pre_delay_read_index: 0,
            pre_delay_write_index: 0,
            detector_average: 0.0,
            compressor_gain: 1.0,
            max_attack_compression_diff_db: -1.0,
            division_remaining: 0,
            envelope_rate: 1.0,
            linear_post_gain: 1.0,
            scaled_desired_gain: 0.0,
            slope: -1.0,
            linear_threshold: -1.0,
            threshold_db: f32::NAN,
            knee_db: f32::NAN,
            knee_threshold: -1.0,
            knee_threshold_db: -1.0,
            knee_output_db: -1.0,
            knee: -1.0,
        };
        compressor.configure_pre_delay();
        compressor.update(parameters);
        compressor
    }

    pub(crate) fn update(&mut self, parameters: CompressorParameters) {
        self.parameters = parameters;
        let knee = self.update_static_curve();
        self.linear_post_gain = self.saturate(1.0, knee).recip().powf(0.6);
    }

    pub(crate) fn process_frame(&mut self, frame: &mut [f32]) {
        debug_assert_eq!(frame.len(), self.channels);
        if self.division_remaining == 0 {
            self.update_envelope_rate();
            self.division_remaining = DIVISION_FRAMES;
        }

        let mut compressor_input = 0.0_f32;
        for (channel, sample) in frame.iter().copied().enumerate() {
            self.pre_delay_buffers[channel][self.pre_delay_write_index] = sample;
            compressor_input = compressor_input.max(sample.abs());
        }

        let shaped_input = self.saturate(compressor_input, self.knee);
        let attenuation = if compressor_input <= 0.0001 {
            1.0
        } else {
            shaped_input / compressor_input
        };
        let attenuation_db = (-linear_to_db(attenuation)).max(2.0);
        let db_per_frame = attenuation_db / (SAT_RELEASE_SECONDS * self.sample_rate);
        let sat_release_rate = db_to_linear(db_per_frame) - 1.0;
        let rate = if attenuation > self.detector_average {
            sat_release_rate
        } else {
            1.0
        };
        self.detector_average += (attenuation - self.detector_average) * rate;
        self.detector_average = self.detector_average.min(1.0);
        if !self.detector_average.is_finite() {
            self.detector_average = 1.0;
        }

        if self.envelope_rate < 1.0 {
            self.compressor_gain +=
                (self.scaled_desired_gain - self.compressor_gain) * self.envelope_rate;
        } else {
            self.compressor_gain = (self.compressor_gain * self.envelope_rate).min(1.0);
        }
        let warped_gain = (FRAC_PI_2 * self.compressor_gain).sin();
        let total_gain = self.linear_post_gain * warped_gain;

        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = self.pre_delay_buffers[channel][self.pre_delay_read_index] * total_gain;
        }
        self.pre_delay_read_index = (self.pre_delay_read_index + 1) & PRE_DELAY_MASK;
        self.pre_delay_write_index = (self.pre_delay_write_index + 1) & PRE_DELAY_MASK;
        self.division_remaining -= 1;
    }

    pub(crate) fn reset(&mut self) {
        self.detector_average = 0.0;
        self.compressor_gain = 1.0;
        self.max_attack_compression_diff_db = -1.0;
        self.division_remaining = 0;
        for buffer in &mut self.pre_delay_buffers {
            buffer.fill(0.0);
        }
        self.configure_pre_delay();
    }

    fn configure_pre_delay(&mut self) {
        let frames = ((PRE_DELAY_SECONDS * self.sample_rate) as usize).min(PRE_DELAY_MASK);
        self.pre_delay_read_index = 0;
        self.pre_delay_write_index = frames;
    }

    fn update_envelope_rate(&mut self) {
        if !self.detector_average.is_finite() {
            self.detector_average = 1.0;
        }
        let desired_gain = self.detector_average;
        let scaled_desired_gain = desired_gain.asin() / FRAC_PI_2;
        self.scaled_desired_gain = scaled_desired_gain;
        let releasing = scaled_desired_gain > self.compressor_gain;
        let mut compression_diff_db = if scaled_desired_gain == 0.0 {
            if releasing { -1.0 } else { 1.0 }
        } else {
            linear_to_db(self.compressor_gain / scaled_desired_gain)
        };

        if releasing {
            self.max_attack_compression_diff_db = -1.0;
            if !compression_diff_db.is_finite() {
                compression_diff_db = -1.0;
            }
            let x = 0.25 * (compression_diff_db.clamp(-12.0, 0.0) + 12.0);
            let x2 = x * x;
            let x3 = x2 * x;
            let x4 = x2 * x2;
            let release_frames = self.sample_rate * self.parameters.release_seconds;
            let a = release_frames * release_a_base();
            let b = release_frames * release_b_base();
            let c = release_frames * release_c_base();
            let d = release_frames * release_d_base();
            let e = release_frames * release_e_base();
            let adaptive_frames = a + b * x + c * x2 + d * x3 + e * x4;
            self.envelope_rate = db_to_linear(5.0 / adaptive_frames);
        } else {
            if !compression_diff_db.is_finite() {
                compression_diff_db = 1.0;
            }
            if self.max_attack_compression_diff_db == -1.0
                || self.max_attack_compression_diff_db < compression_diff_db
            {
                self.max_attack_compression_diff_db = compression_diff_db;
            }
            let effective_diff = self.max_attack_compression_diff_db.max(0.5);
            let attack_frames = self.parameters.attack_seconds.max(0.001) * self.sample_rate;
            self.envelope_rate = 1.0 - (0.25 / effective_diff).powf(1.0 / attack_frames);
        }
    }

    fn knee_curve(&self, input: f32, knee: f32) -> f32 {
        if input < self.linear_threshold {
            input
        } else {
            self.linear_threshold
                + (1.0 - f64::from(-knee * (input - self.linear_threshold)).exp() as f32) / knee
        }
    }

    fn saturate(&self, input: f32, knee: f32) -> f32 {
        if input < self.knee_threshold {
            self.knee_curve(input, knee)
        } else {
            let input_db = linear_to_db(input);
            db_to_linear(self.knee_output_db + self.slope * (input_db - self.knee_threshold_db))
        }
    }

    fn knee_at_slope(&self, desired_slope: f32) -> f32 {
        let input = db_to_linear(self.threshold_db + self.knee_db);
        let (input_2, input_2_db) = if input < self.linear_threshold {
            (1.0, 0.0)
        } else {
            let value = input * 1.001;
            (value, linear_to_db(value))
        };
        let mut min_k = 0.1_f32;
        let mut max_k = 10_000.0_f32;
        let mut knee = 5.0_f32;
        for _ in 0..15 {
            let mut slope = 1.0;
            if input >= self.linear_threshold {
                let output_db = linear_to_db(self.knee_curve(input, knee));
                let output_2_db = linear_to_db(self.knee_curve(input_2, knee));
                slope = (output_2_db - output_db) / (input_2_db - linear_to_db(input));
            }
            if slope < desired_slope {
                max_k = knee;
            } else {
                min_k = knee;
            }
            knee = (min_k * max_k).sqrt();
        }
        knee
    }

    fn update_static_curve(&mut self) -> f32 {
        if self.threshold_db != self.parameters.threshold_db
            || self.knee_db != self.parameters.knee_db
            || self.slope != self.parameters.ratio.recip()
        {
            self.threshold_db = self.parameters.threshold_db;
            self.linear_threshold = db_to_linear(self.threshold_db);
            self.knee_db = self.parameters.knee_db;
            self.slope = self.parameters.ratio.recip();
            let knee = self.knee_at_slope(self.slope);
            self.knee_threshold_db = self.threshold_db + self.knee_db;
            self.knee_threshold = db_to_linear(self.knee_threshold_db);
            self.knee_output_db = linear_to_db(self.knee_curve(self.knee_threshold, knee));
            self.knee = knee;
        }
        self.knee
    }
}

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn linear_to_db(linear: f32) -> f32 {
    20.0 * linear.log10()
}

fn release_a_base() -> f32 {
    0.999_999_999_999_999_8 * RELEASE_ZONE_1 + 1.843_222e-16 * RELEASE_ZONE_2
        - 1.937_339_4e-16 * RELEASE_ZONE_3
        + 8.824_516e-18 * RELEASE_ZONE_4
}

fn release_b_base() -> f32 {
    -1.578_832 * RELEASE_ZONE_1 + 2.330_583_8 * RELEASE_ZONE_2 - 0.914_119_4 * RELEASE_ZONE_3
        + 0.162_367_75 * RELEASE_ZONE_4
}

fn release_c_base() -> f32 {
    0.533_414_3 * RELEASE_ZONE_1 - 1.272_736_8 * RELEASE_ZONE_2 + 0.925_885_6 * RELEASE_ZONE_3
        - 0.186_563_1 * RELEASE_ZONE_4
}

fn release_d_base() -> f32 {
    0.087_834_634 * RELEASE_ZONE_1 - 0.169_416_3 * RELEASE_ZONE_2 + 0.085_880_58 * RELEASE_ZONE_3
        - 0.004_298_914 * RELEASE_ZONE_4
}

fn release_e_base() -> f32 {
    -0.042_416_885 * RELEASE_ZONE_1 + 0.111_569_38 * RELEASE_ZONE_2 - 0.097_646_765 * RELEASE_ZONE_3
        + 0.028_494_263 * RELEASE_ZONE_4
}

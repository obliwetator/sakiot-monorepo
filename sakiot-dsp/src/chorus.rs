use std::f64::consts::PI;

use crate::{
    SegmentEffects, WEB_AUDIO_DELAY_FRACTION_STEPS, WEB_AUDIO_RENDER_QUANTUM_FRAMES,
    equal_power_gains,
};

#[derive(Debug)]
struct ChorusChannel {
    delay_buffer: Vec<f32>,
    write_index: usize,
    feedback_buffer: [f32; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
    feedback_index: usize,
}

#[derive(Debug)]
pub(crate) struct Chorus {
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
    pub(crate) fn new(sample_rate: f64, channel_count: usize, effects: SegmentEffects) -> Self {
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

    pub(crate) fn update(&mut self, effects: SegmentEffects) {
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

    pub(crate) fn process_frame(&mut self, frame: &mut [f32]) {
        let time = self.frame_index as f64 / self.sample_rate;
        for (channel_index, sample) in frame.iter_mut().enumerate() {
            let phase_degrees = if channel_index % 2 == 0 {
                90.0 - f64::from(self.spread_degrees) / 2.0
            } else {
                90.0 + f64::from(self.spread_degrees) / 2.0
            };
            let phase = phase_degrees * PI / 180.0;
            let lfo = libm::sin(2.0 * PI * f64::from(self.frequency_hz) * time - phase);
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

    pub(crate) fn reset(&mut self) {
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

use crate::{WEB_AUDIO_DELAY_FRACTION_STEPS, WEB_AUDIO_RENDER_QUANTUM_FRAMES, equal_power_gains};

#[derive(Debug)]
struct DelayChannel {
    buffer: Vec<f32>,
    write_index: usize,
    feedback_buffer: [f32; WEB_AUDIO_RENDER_QUANTUM_FRAMES],
    feedback_index: usize,
}

#[derive(Debug)]
pub(crate) struct FeedbackDelay {
    channels: Vec<DelayChannel>,
    delay_samples: f64,
    feedback: f32,
    dry_gain: f32,
    wet_gain: f32,
}

impl FeedbackDelay {
    pub(crate) fn new(
        sample_rate: f64,
        channels: usize,
        delay_seconds: f32,
        feedback: f32,
        wet: f32,
    ) -> Self {
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

    pub(crate) fn update(&mut self, sample_rate: f64, delay_seconds: f32, feedback: f32, wet: f32) {
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

    pub(crate) fn process(&mut self, channel: usize, input: f32) -> f32 {
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

    pub(crate) fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.buffer.fill(0.0);
            channel.write_index = 0;
            channel.feedback_buffer.fill(0.0);
            channel.feedback_index = 0;
        }
    }
}

fn web_audio_delay_samples(sample_rate: f64, delay_seconds: f32) -> f64 {
    // Chromium's DelayNode uses 8 fractional bits for its interpolated read
    // position. Matching that 1/256-frame quantization removes a small but
    // measurable difference for non-integer delay times.
    ((sample_rate * f64::from(delay_seconds) * WEB_AUDIO_DELAY_FRACTION_STEPS).round()
        / WEB_AUDIO_DELAY_FRACTION_STEPS)
        .max(1.0)
}

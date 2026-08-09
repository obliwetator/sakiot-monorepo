use crate::equal_power_gains;
use crate::fft::{Complex, transform};

pub(crate) const REVERB_PARTITION_FRAMES: usize = 128;
const REVERB_EARLY_FRAMES: usize = 2_048;
const REVERB_LATE_PARTITION_FRAMES: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReverbParameters {
    pub(crate) enabled: bool,
    pub(crate) decay_seconds: f32,
    pub(crate) pre_delay_seconds: f32,
    pub(crate) wet: f32,
    pub(crate) seed: u32,
}

#[derive(Debug)]
struct PartitionedConvolver<const BLOCK_FRAMES: usize> {
    impulse_spectra: Vec<Vec<Complex>>,
    input_spectra: Vec<Vec<Complex>>,
    spectrum_index: usize,
    input: [f32; BLOCK_FRAMES],
    output: [f32; BLOCK_FRAMES],
    overlap: [f32; BLOCK_FRAMES],
    frame_index: usize,
    scratch: Vec<Complex>,
}

#[derive(Debug)]
struct ReverbChannel {
    head_impulse: Vec<f32>,
    head_history: Vec<f32>,
    head_index: usize,
    early_tail: PartitionedConvolver<REVERB_PARTITION_FRAMES>,
    late_tail: PartitionedConvolver<REVERB_LATE_PARTITION_FRAMES>,
}

impl ReverbChannel {
    fn new(impulse: &[f32]) -> Self {
        let head_length = impulse.len().min(REVERB_PARTITION_FRAMES);
        let head_impulse = impulse[..head_length].to_vec();
        let early_end = impulse.len().min(REVERB_EARLY_FRAMES);
        let early_impulse = impulse
            .get(REVERB_PARTITION_FRAMES..early_end)
            .unwrap_or(&[0.0]);
        // This convolver adds one block of latency. A 1,024-sample zero prefix
        // therefore places the late IR's first sample back at frame 2,048.
        let mut late_impulse = vec![0.0; REVERB_LATE_PARTITION_FRAMES];
        if impulse.len() > REVERB_EARLY_FRAMES {
            late_impulse.extend_from_slice(&impulse[REVERB_EARLY_FRAMES..]);
        }
        Self {
            head_history: vec![0.0; head_length.max(1)],
            head_impulse,
            head_index: 0,
            early_tail: PartitionedConvolver::new(early_impulse),
            late_tail: PartitionedConvolver::new(&late_impulse),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.head_history[self.head_index] = input;
        let length = self.head_history.len();
        let mut head = 0.0;
        for (tap, coefficient) in self.head_impulse.iter().copied().enumerate() {
            head += coefficient * self.head_history[(self.head_index + length - tap) % length];
        }
        self.head_index = (self.head_index + 1) % length;
        head + self.early_tail.process(input) + self.late_tail.process(input)
    }

    fn reset(&mut self) {
        self.head_history.fill(0.0);
        self.head_index = 0;
        self.early_tail.reset();
        self.late_tail.reset();
    }
}

impl<const BLOCK_FRAMES: usize> PartitionedConvolver<BLOCK_FRAMES> {
    fn new(impulse: &[f32]) -> Self {
        let fft_frames = BLOCK_FRAMES * 2;
        let partition_count = impulse.len().div_ceil(BLOCK_FRAMES).max(1);
        let mut impulse_spectra = Vec::with_capacity(partition_count);
        for partition in 0..partition_count {
            let mut spectrum = vec![Complex::default(); fft_frames];
            let start = partition * BLOCK_FRAMES;
            let end = (start + BLOCK_FRAMES).min(impulse.len());
            for (target, source) in spectrum.iter_mut().zip(&impulse[start..end]) {
                target.re = *source;
            }
            transform(&mut spectrum, false);
            impulse_spectra.push(spectrum);
        }
        Self {
            input_spectra: vec![vec![Complex::default(); fft_frames]; partition_count],
            impulse_spectra,
            spectrum_index: 0,
            input: [0.0; BLOCK_FRAMES],
            output: [0.0; BLOCK_FRAMES],
            overlap: [0.0; BLOCK_FRAMES],
            frame_index: 0,
            scratch: vec![Complex::default(); fft_frames],
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.output[self.frame_index];
        self.input[self.frame_index] = input;
        self.frame_index += 1;
        if self.frame_index == BLOCK_FRAMES {
            self.process_partition();
            self.frame_index = 0;
        }
        output
    }

    fn process_partition(&mut self) {
        let spectrum = &mut self.input_spectra[self.spectrum_index];
        spectrum.fill(Complex::default());
        for (value, input) in spectrum.iter_mut().zip(self.input) {
            value.re = input;
        }
        transform(spectrum, false);

        self.scratch.fill(Complex::default());
        let count = self.impulse_spectra.len();
        for (partition, impulse) in self.impulse_spectra.iter().enumerate() {
            let input_index = (self.spectrum_index + count - partition) % count;
            for ((sum, impulse_bin), input_bin) in self
                .scratch
                .iter_mut()
                .zip(impulse)
                .zip(&self.input_spectra[input_index])
            {
                let product = impulse_bin.multiply(*input_bin);
                sum.re += product.re;
                sum.im += product.im;
            }
        }
        transform(&mut self.scratch, true);
        for frame in 0..BLOCK_FRAMES {
            self.output[frame] = self.scratch[frame].re + self.overlap[frame];
            self.overlap[frame] = self.scratch[frame + BLOCK_FRAMES].re;
        }
        self.spectrum_index = (self.spectrum_index + 1) % count;
    }

    fn reset(&mut self) {
        for spectrum in &mut self.input_spectra {
            spectrum.fill(Complex::default());
        }
        self.spectrum_index = 0;
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.overlap.fill(0.0);
        self.frame_index = 0;
        self.scratch.fill(Complex::default());
    }
}

#[derive(Debug)]
pub(crate) struct Reverb {
    sample_rate: f64,
    channel_count: usize,
    parameters: ReverbParameters,
    channels: Vec<ReverbChannel>,
    dry_gain: f32,
    wet_gain: f32,
}

impl Reverb {
    pub(crate) fn new(
        sample_rate: f64,
        channel_count: usize,
        parameters: ReverbParameters,
    ) -> Self {
        let mut reverb = Self {
            sample_rate,
            channel_count,
            parameters,
            channels: Vec::with_capacity(channel_count),
            dry_gain: 1.0,
            wet_gain: 0.0,
        };
        (reverb.dry_gain, reverb.wet_gain) = equal_power_gains(parameters.wet);
        if parameters.enabled {
            reverb.rebuild();
        }
        reverb
    }

    pub(crate) fn update(&mut self, parameters: ReverbParameters) {
        let impulse_changed = self.parameters.decay_seconds != parameters.decay_seconds
            || self.parameters.pre_delay_seconds != parameters.pre_delay_seconds
            || self.parameters.seed != parameters.seed;
        let needs_impulse = parameters.enabled && (impulse_changed || self.channels.is_empty());
        self.parameters = parameters;
        (self.dry_gain, self.wet_gain) = equal_power_gains(parameters.wet);
        if needs_impulse {
            self.rebuild();
        }
    }

    pub(crate) fn process_frame(&mut self, frame: &mut [f32]) {
        if !self.parameters.enabled {
            return;
        }
        for (channel_index, sample) in frame.iter_mut().enumerate() {
            let input = *sample;
            let wet = self.channels[channel_index].process(input);
            *sample = input * self.dry_gain + wet * self.wet_gain;
        }
    }

    pub(crate) fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
    }

    fn rebuild(&mut self) {
        self.channels = (0..self.channel_count)
            .map(|channel| {
                let seed = self.parameters.seed ^ (channel as u32).wrapping_mul(0x9e37_79b9);
                ReverbChannel::new(&generate_impulse(
                    self.sample_rate,
                    self.parameters.decay_seconds,
                    self.parameters.pre_delay_seconds,
                    seed,
                ))
            })
            .collect();
        (self.dry_gain, self.wet_gain) = equal_power_gains(self.parameters.wet);
        self.reset();
    }
}

fn generate_impulse(
    sample_rate: f64,
    decay_seconds: f32,
    pre_delay_seconds: f32,
    mut state: u32,
) -> Vec<f32> {
    if state == 0 {
        state = 0x6d2b_79f5;
    }
    let pre_delay_frames = (sample_rate * f64::from(pre_delay_seconds)).round() as usize;
    let decay_frames = (sample_rate * f64::from(decay_seconds)).ceil() as usize;
    let mut impulse = vec![0.0; pre_delay_frames + decay_frames.max(1)];
    let time_constant = f64::from((decay_seconds + 1.0).ln()) / 200.0_f64.ln();
    for frame in 0..decay_frames {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = (state as f64 / u32::MAX as f64) * 2.0 - 1.0;
        let time = frame as f64 / sample_rate;
        let progress = time / f64::from(decay_seconds);
        let envelope = if progress >= 0.9 {
            (1.0 - progress).max(0.0) / 0.1
        } else {
            (-time / time_constant).exp()
        };
        impulse[pre_delay_frames + frame] = (noise * envelope) as f32;
    }

    // ConvolverNode normalizes generated responses. Energy normalization gives
    // this deterministic replacement stable perceived loudness across decay.
    let energy = impulse
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        .sqrt();
    if energy > 0.0 {
        let scale = (1.0 / energy) as f32;
        for sample in &mut impulse {
            *sample *= scale;
        }
    }
    impulse
}

#[cfg(test)]
mod tests {
    use super::ReverbChannel;

    #[test]
    fn non_uniform_schedule_reconstructs_the_original_impulse_timing() {
        let impulse: Vec<f32> = (0..6_000)
            .map(|frame| ((frame as f32 * 0.173).sin() * (-frame as f32 / 2_000.0).exp()) * 0.1)
            .collect();
        let mut convolver = ReverbChannel::new(&impulse);
        let mut output = Vec::with_capacity(impulse.len());
        for frame in 0..impulse.len() {
            output.push(convolver.process(if frame == 0 { 1.0 } else { 0.0 }));
        }
        let max_error = output
            .iter()
            .zip(&impulse)
            .map(|(actual, expected)| (actual - expected).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_error < 2e-5,
            "max impulse reconstruction error {max_error}"
        );
    }
}

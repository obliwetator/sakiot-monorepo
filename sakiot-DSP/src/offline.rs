//! Length-changing clip transforms. These run before the streaming effect
//! chain because playback rate changes the number of frames and reverse needs
//! random access to the complete source.

use crate::fft::{Complex, transform};
use crate::{DspError, SegmentEffects, SegmentProcessor};
use std::f32::consts::{PI, TAU};

const VOCODER_FFT_FRAMES: usize = 2_048;
const VOCODER_SYNTHESIS_HOP: usize = 512;
const RESAMPLER_RADIUS: isize = 24;

pub fn render_clip_interleaved(
    input: &[f32],
    sample_rate: f64,
    channels: usize,
    effects: SegmentEffects,
) -> Result<Vec<f32>, DspError> {
    if !input.len().is_multiple_of(channels.max(1)) {
        return Err(DspError::MisalignedInterleavedBuffer);
    }
    // Constructing this processor performs the common boundary validation.
    let mut streaming_effects = effects;
    streaming_effects.pitch_cents = 0.0;
    streaming_effects.rate = 1.0;
    streaming_effects.reverse = false;
    let mut processor = SegmentProcessor::new(sample_rate, channels, streaming_effects)?;

    let mut transformed = input.to_vec();
    if effects.reverse {
        reverse_interleaved_frames(&mut transformed, channels);
    }

    let pitch_ratio = 2.0_f32.powf(effects.pitch_cents / 1_200.0);
    let stretch = pitch_ratio / effects.rate;
    if (stretch - 1.0).abs() > f32::EPSILON && transformed.len() >= channels * 2 {
        transformed = phase_vocoder_stretch(&transformed, channels, stretch);
    }
    if (pitch_ratio - 1.0).abs() > f32::EPSILON {
        transformed = resample_interleaved(&transformed, channels, pitch_ratio);
    }
    let target_frames =
        ((input.len() / channels) as f64 / f64::from(effects.rate)).round() as usize;
    let tail_frames = (f64::from(effects.tail_seconds) * sample_rate).round() as usize;
    transformed.resize((target_frames + tail_frames) * channels, 0.0);
    processor.process_interleaved(&mut transformed)?;
    Ok(transformed)
}

pub fn reverse_interleaved_frames(samples: &mut [f32], channels: usize) {
    let frames = samples.len() / channels;
    for left in 0..frames / 2 {
        let right = frames - 1 - left;
        for channel in 0..channels {
            samples.swap(left * channels + channel, right * channels + channel);
        }
    }
}

fn phase_vocoder_stretch(input: &[f32], channels: usize, stretch: f32) -> Vec<f32> {
    let input_frames = input.len() / channels;
    let padding = VOCODER_FFT_FRAMES / 2;
    let padded_frames = input_frames + padding * 2;
    let analysis_hop = VOCODER_SYNTHESIS_HOP as f32 / stretch;
    let frame_count =
        (((padded_frames - VOCODER_FFT_FRAMES) as f32 / analysis_hop).ceil() as usize) + 1;
    let synthesis_frames = (frame_count - 1) * VOCODER_SYNTHESIS_HOP + VOCODER_FFT_FRAMES;
    let mut output = vec![0.0; synthesis_frames * channels];
    let mut normalization = vec![0.0; synthesis_frames];
    let window: Vec<f32> = (0..VOCODER_FFT_FRAMES)
        .map(|frame| 0.5 - 0.5 * (TAU * frame as f32 / VOCODER_FFT_FRAMES as f32).cos())
        .collect();

    for channel in 0..channels {
        let mut previous_phase = vec![0.0; VOCODER_FFT_FRAMES / 2 + 1];
        let mut synthesis_phase = vec![0.0; VOCODER_FFT_FRAMES / 2 + 1];
        let mut spectrum = vec![Complex::default(); VOCODER_FFT_FRAMES];
        for analysis_frame in 0..frame_count {
            let analysis_position = analysis_frame as f32 * analysis_hop;
            for bin in 0..VOCODER_FFT_FRAMES {
                let padded_position = analysis_position + bin as f32;
                let source_position = padded_position - padding as f32;
                spectrum[bin] = Complex {
                    re: sample_interleaved_linear(input, channels, channel, source_position)
                        * window[bin],
                    im: 0.0,
                };
            }
            transform(&mut spectrum, false);

            for bin in 0..=VOCODER_FFT_FRAMES / 2 {
                let magnitude = spectrum[bin].magnitude();
                let phase = spectrum[bin].phase();
                if analysis_frame == 0 {
                    synthesis_phase[bin] = phase;
                } else {
                    let expected = TAU * bin as f32 * analysis_hop / VOCODER_FFT_FRAMES as f32;
                    let deviation = wrap_phase(phase - previous_phase[bin] - expected);
                    let angular_frequency =
                        TAU * bin as f32 / VOCODER_FFT_FRAMES as f32 + deviation / analysis_hop;
                    synthesis_phase[bin] += angular_frequency * VOCODER_SYNTHESIS_HOP as f32;
                }
                previous_phase[bin] = phase;
                spectrum[bin] = Complex::from_polar(magnitude, synthesis_phase[bin]);
                if bin > 0 && bin < VOCODER_FFT_FRAMES / 2 {
                    spectrum[VOCODER_FFT_FRAMES - bin] = Complex {
                        re: spectrum[bin].re,
                        im: -spectrum[bin].im,
                    };
                }
            }
            transform(&mut spectrum, true);
            let synthesis_position = analysis_frame * VOCODER_SYNTHESIS_HOP;
            for frame in 0..VOCODER_FFT_FRAMES {
                output[(synthesis_position + frame) * channels + channel] +=
                    spectrum[frame].re * window[frame];
                if channel == 0 {
                    normalization[synthesis_position + frame] += window[frame] * window[frame];
                }
            }
        }
    }

    for frame in 0..synthesis_frames {
        let scale = if normalization[frame] > 1e-8 {
            normalization[frame].recip()
        } else {
            0.0
        };
        for channel in 0..channels {
            output[frame * channels + channel] *= scale;
        }
    }

    let crop_start = (padding as f32 * stretch).round() as usize;
    let wanted_frames = (input_frames as f32 * stretch).round() as usize;
    let crop_end = (crop_start + wanted_frames).min(synthesis_frames);
    output[crop_start * channels..crop_end * channels].to_vec()
}

fn resample_interleaved(input: &[f32], channels: usize, speed: f32) -> Vec<f32> {
    let input_frames = input.len() / channels;
    let output_frames = (input_frames as f64 / f64::from(speed)).round() as usize;
    let mut output = vec![0.0; output_frames * channels];
    let cutoff = if speed > 1.0 { speed.recip() } else { 1.0 };
    for output_frame in 0..output_frames {
        let source_position = output_frame as f64 * f64::from(speed);
        let center = source_position.floor() as isize;
        for channel in 0..channels {
            let mut sum = 0.0_f64;
            let mut weight_sum = 0.0_f64;
            for tap in -RESAMPLER_RADIUS + 1..=RESAMPLER_RADIUS {
                let source_frame = center + tap;
                if source_frame < 0 || source_frame >= input_frames as isize {
                    continue;
                }
                let distance = source_position - source_frame as f64;
                let normalized = distance / RESAMPLER_RADIUS as f64;
                let window = if normalized.abs() <= 1.0 {
                    0.42 + 0.5 * (PI as f64 * normalized).cos()
                        + 0.08 * (2.0 * PI as f64 * normalized).cos()
                } else {
                    0.0
                };
                let sinc_position = distance * f64::from(cutoff);
                let sinc = if sinc_position.abs() < 1e-12 {
                    1.0
                } else {
                    (PI as f64 * sinc_position).sin() / (PI as f64 * sinc_position)
                };
                let weight = window * sinc * f64::from(cutoff);
                sum += f64::from(input[source_frame as usize * channels + channel]) * weight;
                weight_sum += weight;
            }
            if weight_sum.abs() > 1e-12 {
                output[output_frame * channels + channel] = (sum / weight_sum) as f32;
            }
        }
    }
    output
}

fn sample_interleaved_linear(input: &[f32], channels: usize, channel: usize, position: f32) -> f32 {
    let frames = input.len() / channels;
    if position < 0.0 || position >= frames as f32 {
        return 0.0;
    }
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(frames - 1);
    let fraction = position - lower as f32;
    let a = input[lower * channels + channel];
    let b = input[upper * channels + channel];
    a + (b - a) * fraction
}

fn wrap_phase(mut phase: f32) -> f32 {
    while phase > PI {
        phase -= TAU;
    }
    while phase < -PI {
        phase += TAU;
    }
    phase
}

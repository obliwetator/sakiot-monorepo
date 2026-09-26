//! Bounded-memory length-changing rendering. Source length is known before
//! processing (a decoded PCM file on the server). Reverse is performed by the
//! caller, supplying frames in reverse order, never reversing channel order.
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use crate::fft::{Complex, transform};
use crate::{DspError, SegmentEffects, SegmentProcessor};

const FFT: usize = 2048;
const HOP: usize = 512;
const RADIUS: isize = 24;

struct Frames {
    samples: VecDeque<f32>,
    base: usize,
    channels: usize,
}
impl Frames {
    fn new(channels: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            base: 0,
            channels,
        }
    }
    fn end(&self) -> usize {
        self.base + self.samples.len() / self.channels
    }
    fn get(&self, frame: usize, channel: usize) -> f32 {
        self.samples[(frame - self.base) * self.channels + channel]
    }
    fn discard_before(&mut self, frame: usize) {
        let count = frame.min(self.end()).saturating_sub(self.base);
        self.samples.drain(..count * self.channels);
        self.base += count;
    }
}

/// A segment owns one processor for its entire lifetime. `push` accepts any
/// aligned block size and invokes the sink with bounded output blocks. The sink
/// must consume each block before returning. `finish` is required exactly once.
/// Reverse input must already be ordered backwards by frame; passing reverse
/// here is rejected to prevent accidentally exporting forward audio.
pub struct IncrementalRenderer {
    channels: usize,
    input_frames: usize,
    received: usize,
    source: Frames,
    stretched: Frames,
    pitch: f32,
    vocoder: bool,
    analysis_hop: f32,
    analysis_frame: usize,
    analysis_count: usize,
    crop_start: usize,
    crop_end: usize,
    transformed_frames: usize,
    resampled_frames: usize,
    output_frames: usize,
    output_index: usize,
    window: Vec<f32>,
    previous_phase: Vec<Vec<f32>>,
    synthesis_phase: Vec<Vec<f32>>,
    spectrum: Vec<Complex>,
    overlap: Vec<f32>,
    normalization: Vec<f32>,
    processor: SegmentProcessor,
    output: Vec<f32>,
    finished: bool,
}

impl IncrementalRenderer {
    pub fn new(
        sample_rate: f64,
        channels: usize,
        input_frames: usize,
        effects: SegmentEffects,
    ) -> Result<Self, DspError> {
        crate::validate_effects(effects)?;
        if effects.reverse {
            return Err(DspError::ReverseRequiresOrderedInput);
        }
        let mut streaming = effects;
        streaming.pitch_cents = 0.0;
        streaming.rate = 1.0;
        let processor = SegmentProcessor::new(sample_rate, channels, streaming)?;
        let pitch = libm::powf(2.0, effects.pitch_cents / 1200.0);
        let stretch = pitch / effects.rate;
        let vocoder = (stretch - 1.0).abs() > f32::EPSILON && input_frames >= 2;
        let analysis_hop = HOP as f32 / stretch;
        let crop_start = if vocoder {
            (FFT as f32 / 2.0 * stretch).round() as usize
        } else {
            0
        };
        let crop_end = if vocoder {
            crop_start + (input_frames as f64 * f64::from(stretch)).round() as usize
        } else {
            input_frames
        };
        let analysis_count = ((input_frames as f64 / f64::from(analysis_hop)).ceil() as usize + 1)
            .max(crop_end.saturating_sub(FFT).div_ceil(HOP) + 1);
        let transformed_frames = crop_end.saturating_sub(crop_start);
        let resampled_frames = if (pitch - 1.0).abs() > f32::EPSILON {
            (transformed_frames as f64 / f64::from(pitch)).round() as usize
        } else {
            transformed_frames
        };
        let output_frames = (input_frames as f64 / f64::from(effects.rate)).round() as usize
            + (f64::from(effects.tail_seconds) * sample_rate).round() as usize;
        Ok(Self {
            channels,
            input_frames,
            received: 0,
            source: Frames::new(channels),
            stretched: Frames::new(channels),
            pitch,
            vocoder,
            analysis_hop,
            analysis_frame: 0,
            analysis_count,
            crop_start,
            crop_end,
            transformed_frames,
            resampled_frames,
            output_frames,
            output_index: 0,
            window: (0..FFT)
                .map(|i| 0.5 - 0.5 * libm::cosf(TAU * i as f32 / FFT as f32))
                .collect(),
            previous_phase: vec![vec![0.0; FFT / 2 + 1]; channels],
            synthesis_phase: vec![vec![0.0; FFT / 2 + 1]; channels],
            spectrum: vec![Complex::default(); FFT],
            overlap: vec![0.0; FFT * channels],
            normalization: vec![0.0; FFT],
            processor,
            output: Vec::with_capacity(HOP * channels),
            finished: false,
        })
    }

    pub fn output_frames(&self) -> usize {
        self.output_frames
    }

    pub fn push<E>(
        &mut self,
        input: &[f32],
        mut sink: impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<(), StreamError<E>> {
        if self.finished {
            return Err(StreamError::Dsp(DspError::StreamFinished));
        }
        if !input.len().is_multiple_of(self.channels) {
            return Err(StreamError::Dsp(DspError::MisalignedInterleavedBuffer));
        }
        if input.len() / self.channels > self.input_frames - self.received {
            return Err(StreamError::Dsp(DspError::StreamLengthMismatch));
        }
        for block in input.chunks(HOP * self.channels) {
            self.received += block.len() / self.channels;
            if self.vocoder {
                self.source.samples.extend(block);
                self.pump_vocoder(&mut sink)?;
            } else {
                self.stretched.samples.extend(block);
                self.pump_resampler(&mut sink)?;
            }
        }
        self.flush(&mut sink)
    }

    pub fn finish<E>(
        &mut self,
        mut sink: impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<(), StreamError<E>> {
        if self.finished {
            return Err(StreamError::Dsp(DspError::StreamFinished));
        }
        if self.received != self.input_frames {
            return Err(StreamError::Dsp(DspError::StreamLengthMismatch));
        }
        self.finished = true;
        if self.vocoder {
            self.pump_vocoder(&mut sink)?;
            // The last FFT's remaining overlap has no later contributors.
            let start = self.analysis_count * HOP;
            self.emit_overlap(start, FFT - HOP);
            self.pump_resampler(&mut sink)?;
        }
        while self.output_index < self.output_frames {
            for _ in 0..self.channels {
                self.output.push(0.0);
            }
            self.output_index += 1;
            if self.output.len() >= HOP * self.channels {
                self.flush(&mut sink)?;
            }
        }
        self.flush(&mut sink)
    }

    fn pump_vocoder<E>(
        &mut self,
        sink: &mut impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<(), StreamError<E>> {
        while self.analysis_frame < self.analysis_count {
            let position = self.analysis_frame as f64 * f64::from(self.analysis_hop);
            let last = position + (FFT - 1) as f64 - (FFT / 2) as f64;
            let needed = ((last.max(0.0).floor() as usize) + 2).min(self.input_frames);
            if self.received < needed {
                break;
            }
            for channel in 0..self.channels {
                for bin in 0..FFT {
                    let p = position + bin as f64 - (FFT / 2) as f64;
                    let value = if p < 0.0 || p >= self.input_frames as f64 {
                        0.0
                    } else {
                        let lower = p.floor() as usize;
                        let upper = (lower + 1).min(self.input_frames - 1);
                        let a = self.source.get(lower, channel);
                        a + (self.source.get(upper, channel) - a) * (p - lower as f64) as f32
                    };
                    self.spectrum[bin] = Complex {
                        re: value * self.window[bin],
                        im: 0.0,
                    };
                }
                transform(&mut self.spectrum, false);
                for bin in 0..=FFT / 2 {
                    let magnitude = self.spectrum[bin].magnitude();
                    let phase = self.spectrum[bin].phase();
                    if self.analysis_frame == 0 {
                        self.synthesis_phase[channel][bin] = phase;
                    } else {
                        let expected = TAU * bin as f32 * self.analysis_hop / FFT as f32;
                        let mut deviation = phase - self.previous_phase[channel][bin] - expected;
                        while deviation > PI {
                            deviation -= TAU;
                        }
                        while deviation < -PI {
                            deviation += TAU;
                        }
                        let frequency =
                            TAU * bin as f32 / FFT as f32 + deviation / self.analysis_hop;
                        self.synthesis_phase[channel][bin] += frequency * HOP as f32;
                    }
                    self.previous_phase[channel][bin] = phase;
                    self.spectrum[bin] =
                        Complex::from_polar(magnitude, self.synthesis_phase[channel][bin]);
                    if bin > 0 && bin < FFT / 2 {
                        self.spectrum[FFT - bin] = Complex {
                            re: self.spectrum[bin].re,
                            im: -self.spectrum[bin].im,
                        };
                    }
                }
                transform(&mut self.spectrum, true);
                for frame in 0..FFT {
                    self.overlap[frame * self.channels + channel] +=
                        self.spectrum[frame].re * self.window[frame];
                    if channel == 0 {
                        self.normalization[frame] += self.window[frame] * self.window[frame];
                    }
                }
            }
            self.emit_overlap(self.analysis_frame * HOP, HOP);
            self.overlap.copy_within(HOP * self.channels.., 0);
            self.overlap[(FFT - HOP) * self.channels..].fill(0.0);
            self.normalization.copy_within(HOP.., 0);
            self.normalization[FFT - HOP..].fill(0.0);
            self.analysis_frame += 1;
            let next = self.analysis_frame as f64 * f64::from(self.analysis_hop) - (FFT / 2) as f64;
            self.source.discard_before(next.max(0.0).floor() as usize);
            self.pump_resampler(sink)?;
        }
        Ok(())
    }

    fn emit_overlap(&mut self, start: usize, count: usize) {
        for frame in 0..count {
            if start + frame < self.crop_start || start + frame >= self.crop_end {
                continue;
            }
            let scale = if self.normalization[frame] > 1e-8 {
                self.normalization[frame].recip()
            } else {
                0.0
            };
            for channel in 0..self.channels {
                self.stretched
                    .samples
                    .push_back(self.overlap[frame * self.channels + channel] * scale);
            }
        }
    }

    fn pump_resampler<E>(
        &mut self,
        sink: &mut impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<(), StreamError<E>> {
        let resample = (self.pitch - 1.0).abs() > f32::EPSILON;
        let cutoff = if self.pitch > 1.0 {
            self.pitch.recip()
        } else {
            1.0
        };
        while self.output_index < self.resampled_frames.min(self.output_frames) {
            let position = self.output_index as f64 * f64::from(self.pitch);
            let center = position.floor() as isize;
            let needed = if resample {
                (center + RADIUS + 1).max(0) as usize
            } else {
                self.output_index + 1
            };
            if self.stretched.end() < needed.min(self.transformed_frames) {
                break;
            }
            for channel in 0..self.channels {
                let sample = if !resample {
                    self.stretched.get(self.output_index, channel)
                } else {
                    let mut sum = 0.0_f64;
                    let mut weight_sum = 0.0_f64;
                    for tap in -RADIUS + 1..=RADIUS {
                        let frame = center + tap;
                        if frame < 0 || frame >= self.transformed_frames as isize {
                            continue;
                        }
                        let distance = position - frame as f64;
                        let normalized = distance / RADIUS as f64;
                        let window = if normalized.abs() <= 1.0 {
                            0.42 + 0.5 * libm::cos(PI as f64 * normalized)
                                + 0.08 * libm::cos(2.0 * PI as f64 * normalized)
                        } else {
                            0.0
                        };
                        let p = distance * f64::from(cutoff);
                        let sinc = if p.abs() < 1e-12 {
                            1.0
                        } else {
                            libm::sin(PI as f64 * p) / (PI as f64 * p)
                        };
                        let weight = window * sinc * f64::from(cutoff);
                        sum += f64::from(self.stretched.get(frame as usize, channel)) * weight;
                        weight_sum += weight;
                    }
                    if weight_sum.abs() > 1e-12 {
                        (sum / weight_sum) as f32
                    } else {
                        0.0
                    }
                };
                self.output.push(sample);
            }
            self.output_index += 1;
            let keep = if resample {
                (self.output_index as f64 * f64::from(self.pitch)).floor() as isize - RADIUS + 1
            } else {
                self.output_index as isize
            };
            self.stretched.discard_before(keep.max(0) as usize);
            if self.output.len() >= HOP * self.channels {
                self.flush(sink)?;
            }
        }
        // Rounding may produce frames beyond the requested segment length.
        if self.output_index >= self.resampled_frames.min(self.output_frames) {
            self.stretched.discard_before(self.stretched.end());
        }
        Ok(())
    }

    fn flush<E>(
        &mut self,
        sink: &mut impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<(), StreamError<E>> {
        if self.output.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.processor.process_interleaved(&mut self.output) {
            self.finished = true;
            return Err(StreamError::Dsp(error));
        }
        if let Err(error) = sink(&self.output) {
            self.finished = true;
            return Err(StreamError::Sink(error));
        }
        self.output.clear();
        Ok(())
    }
}

#[derive(Debug)]
pub enum StreamError<E> {
    Dsp(DspError),
    Sink(E),
}
impl<E: std::fmt::Display> std::fmt::Display for StreamError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp(e) => e.fmt(f),
            Self::Sink(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for StreamError<E> {}

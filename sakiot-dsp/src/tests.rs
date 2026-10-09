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
    let mut processor =
        SegmentProcessor::new(SAMPLE_RATE, 2, SegmentEffects::default()).expect("valid processor");
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
    let mut processor = SegmentProcessor::new(SAMPLE_RATE, 1, effects).expect("valid processor");
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
    let mut bypass =
        SegmentProcessor::new(SAMPLE_RATE, 1, SegmentEffects::default()).expect("valid processor");
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

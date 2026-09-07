use sakiot_dsp::{
    IncrementalRenderer, SegmentEffects, render_clip_interleaved, reverse_interleaved_frames,
};

fn signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            [
                (i as f32 * 0.057).sin() * 0.2,
                (i as f32 * 0.091).cos() * 0.1,
            ]
        })
        .collect()
}
fn compare(input: &[f32], effects: SegmentEffects, block: usize) {
    let reference = render_clip_interleaved(input, 8000.0, 2, effects).unwrap();
    let mut ordered = input.to_vec();
    let mut config = effects;
    if config.reverse {
        reverse_interleaved_frames(&mut ordered, 2);
        config.reverse = false;
    }
    let mut renderer = IncrementalRenderer::new(8000.0, 2, input.len() / 2, config).unwrap();
    let mut actual = Vec::new();
    let mut sink = |samples: &[f32]| {
        assert!(samples.len() <= 1024);
        actual.extend_from_slice(samples);
        Ok::<_, std::convert::Infallible>(())
    };
    for chunk in ordered.chunks(block * 2) {
        renderer.push(chunk, &mut sink).unwrap();
    }
    renderer.finish(&mut sink).unwrap();
    assert_eq!(reference.len(), actual.len());
    let max_error = actual
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    let rms_error = (actual
        .iter()
        .zip(&reference)
        .map(|(a, b)| f64::from(a - b).powi(2))
        .sum::<f64>()
        / actual.len().max(1) as f64)
        .sqrt();
    assert!(
        max_error <= 2e-6 && rms_error <= 2e-7,
        "pitch={} rate={} frames={} block={block} max={max_error} rms={rms_error}",
        effects.pitch_cents,
        effects.rate,
        input.len() / 2
    );
}

#[test]
fn arbitrary_blocks_match_reference_with_all_stateful_effects_and_tail() {
    let effects = SegmentEffects {
        pitch_cents: 700.0,
        rate: 1.35,
        reverse: true,
        volume_db: -2.0,
        bass_db: 2.0,
        mid_db: -3.0,
        treble_db: 1.0,
        distortion_wet: 0.25,
        delay_seconds: 0.03,
        delay_feedback: 0.3,
        delay_wet: 0.4,
        compressor_enabled: true,
        chorus_enabled: true,
        reverb_enabled: true,
        reverb_decay_seconds: 0.1,
        reverb_pre_delay_seconds: 0.01,
        reverb_wet: 0.3,
        tail_seconds: 0.2,
        ..SegmentEffects::default()
    };
    for block in [1, 7, 127, 512, 2049, 100_000] {
        compare(&signal(4001), effects, block);
    }
}

#[test]
fn duration_rounding_and_extreme_pitch_rate_match_reference() {
    for frames in [0, 1, 2, 511, 2048, 2401] {
        for (pitch, rate) in [
            (0.0, 1.0),
            (1200.0, 2.0),
            (-4800.0, 10.0),
            (4800.0, 0.1),
            (0.0, 0.1),
            (0.0, 10.0),
            (-700.0, 0.73),
        ] {
            compare(
                &signal(frames),
                SegmentEffects {
                    pitch_cents: pitch,
                    rate,
                    ..SegmentEffects::default()
                },
                137,
            );
        }
    }
}

#[test]
fn finish_is_explicit_and_rejects_incomplete_or_repeated_input() {
    let mut renderer = IncrementalRenderer::new(8000.0, 2, 2, SegmentEffects::default()).unwrap();
    let sink = |_: &[f32]| Ok::<_, std::convert::Infallible>(());
    assert!(renderer.finish(sink).is_err());
    assert!(renderer.push(&[0.0], sink).is_err());
    assert!(renderer.push(&[0.0; 6], sink).is_err());
    renderer.push(&[0.0; 4], sink).unwrap();
    renderer.finish(sink).unwrap();
    assert!(renderer.finish(sink).is_err());
    assert!(renderer.push(&[], sink).is_err());
    assert!(
        IncrementalRenderer::new(
            8000.0,
            2,
            2,
            SegmentEffects {
                reverse: true,
                ..SegmentEffects::default()
            }
        )
        .is_err()
    );
}

#[test]
fn long_state_history_and_fractional_positions_match_the_reference() {
    compare(
        &signal(65_537),
        SegmentEffects {
            pitch_cents: -700.0,
            rate: 0.73,
            delay_seconds: 0.05,
            delay_feedback: 0.6,
            delay_wet: 0.5,
            chorus_enabled: true,
            reverb_enabled: true,
            reverb_decay_seconds: 0.2,
            reverb_wet: 0.3,
            tail_seconds: 0.25,
            ..SegmentEffects::default()
        },
        4093,
    );
}

#[test]
fn sink_failure_terminates_the_stream() {
    let mut renderer = IncrementalRenderer::new(8000.0, 2, 2, SegmentEffects::default()).unwrap();
    assert!(renderer.push(&[0.0; 4], |_| Err("disk full")).is_err());
    assert!(renderer.finish(|_| Ok::<_, &str>(())).is_err());
}

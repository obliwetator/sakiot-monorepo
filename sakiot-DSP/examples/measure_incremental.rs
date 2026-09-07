//! Reproducible duration-independent-memory benchmark; output is consumed, not
//! retained. Run with /usr/bin/time -v and a release build.
use sakiot_dsp::{IncrementalRenderer, SegmentEffects};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let seconds: usize = args.get(1).map_or(Ok(60), |s| s.parse())?;
    let pitch = args.get(2).map_or(Ok(700.0), |s| s.parse())?;
    let rate = args.get(3).map_or(Ok(1.35), |s| s.parse())?;
    let frames = seconds * 48000;
    let mut effects = SegmentEffects {
        pitch_cents: pitch,
        rate,
        tail_seconds: 0.5,
        distortion_wet: 0.2,
        delay_wet: 0.2,
        compressor_enabled: true,
        chorus_enabled: true,
        reverb_enabled: true,
        reverb_wet: 0.2,
        ..SegmentEffects::default()
    };
    if args.get(4).is_some_and(|arg| arg == "max-effects") {
        effects.tail_seconds = 30.0;
        effects.delay_seconds = 5.0;
        effects.chorus_delay_ms = 100.0;
        effects.reverb_decay_seconds = 30.0;
        effects.reverb_pre_delay_seconds = 5.0;
    }
    let mut renderer = IncrementalRenderer::new(48000.0, 2, frames, effects)?;
    let mut output_frames = 0;
    let mut checksum = 0.0_f64;
    let mut sink = |samples: &[f32]| {
        output_frames += samples.len() / 2;
        checksum += samples.iter().map(|v| f64::from(*v).abs()).sum::<f64>();
        Ok::<_, std::convert::Infallible>(())
    };
    let mut block = vec![0.0; 4096 * 2];
    let start = std::time::Instant::now();
    for base in (0..frames).step_by(4096) {
        let count = (frames - base).min(4096);
        for i in 0..count {
            block[i * 2] = (((base + i) % 48000) as f32 * 0.037).sin() * 0.2;
            block[i * 2 + 1] = (((base + i) % 48000) as f32 * 0.071).cos() * 0.1;
        }
        renderer.push(&block[..count * 2], &mut sink)?;
    }
    renderer.finish(&mut sink)?;
    println!(
        "seconds={seconds} pitch={pitch} rate={rate} output_frames={output_frames} checksum={checksum:.6} elapsed={:.3}s",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

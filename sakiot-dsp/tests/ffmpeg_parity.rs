use sakiot_dsp::{SegmentEffects, SegmentProcessor, db_to_gain, ffmpeg_filter_chain};
use std::error::Error;
use std::f32::consts::TAU;
use std::io::Write;
use std::process::{Command, Stdio};

const SAMPLE_RATE: usize = 48_000;

#[test]
#[ignore = "manual prototype measurement; requires the server's FFmpeg filters"]
fn measures_current_ffmpeg_eq_chain() -> Result<(), Box<dyn Error>> {
    let input = test_signal(SAMPLE_RATE * 2);
    let cases = [
        (
            "volume",
            SegmentEffects {
                volume_db: -3.0,
                ..SegmentEffects::default()
            },
        ),
        (
            "bass",
            SegmentEffects {
                bass_db: 5.0,
                ..SegmentEffects::default()
            },
        ),
        (
            "mid",
            SegmentEffects {
                mid_db: -4.0,
                ..SegmentEffects::default()
            },
        ),
        (
            "treble",
            SegmentEffects {
                treble_db: 2.5,
                ..SegmentEffects::default()
            },
        ),
        (
            "combined",
            SegmentEffects {
                volume_db: -3.0,
                bass_db: 5.0,
                mid_db: -4.0,
                treble_db: 2.5,
                ..SegmentEffects::default()
            },
        ),
    ];
    for (name, effects) in cases {
        let current_filter = current_ffmpeg_filter(effects);
        let measurement = measure(&input, effects, &current_filter)?;
        println!(
            "current {name:>8}: residual={:.2} dBFS, relative={:.2} dB, max_abs={:.8}, correlation={:.9}",
            measurement.residual_dbfs,
            measurement.relative_residual_db,
            measurement.max_abs,
            measurement.correlation,
        );
        // Broad regression guard while this test is a measurement spike. The
        // actual acceptance threshold remains a recorded prototype finding.
        assert!(
            measurement.relative_residual_db < -20.0,
            "shared {name} differs substantially from FFmpeg: {measurement:?}"
        );

        let canonical_filter = ffmpeg_filter_chain(SAMPLE_RATE as f64, effects)?;
        let canonical = measure(&input, effects, &canonical_filter)?;
        println!(
            "shared  {name:>8}: residual={:.2} dBFS, relative={:.2} dB, max_abs={:.8}, correlation={:.9}",
            canonical.residual_dbfs,
            canonical.relative_residual_db,
            canonical.max_abs,
            canonical.correlation,
        );
        assert!(
            canonical.relative_residual_db < -90.0,
            "canonical FFmpeg biquads differ from native: {canonical:?}"
        );
    }
    Ok(())
}

fn measure(
    input: &[f32],
    effects: SegmentEffects,
    filter: &str,
) -> Result<Measurement, Box<dyn Error>> {
    let mut native = input.to_vec();
    SegmentProcessor::new(SAMPLE_RATE as f64, 1, effects)?.process_interleaved(&mut native)?;
    let ffmpeg = render_with_ffmpeg(input, filter)?;
    if ffmpeg.len() != native.len() {
        return Err(format!(
            "FFmpeg returned {} samples for a {} sample input",
            ffmpeg.len(),
            native.len()
        )
        .into());
    }
    // Ignore initial filter startup when reporting the steady-state residual.
    let skip = SAMPLE_RATE / 10;
    Ok(compare(&native[skip..], &ffmpeg[skip..]))
}

fn test_signal(frames: usize) -> Vec<f32> {
    let mut noise = 0x51a9_10d5_u32;
    (0..frames)
        .map(|frame| {
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            let random = (noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let time = frame as f32 / SAMPLE_RATE as f32;
            0.08 * (TAU * 83.0 * time).sin()
                + 0.06 * (TAU * 997.0 * time).sin()
                + 0.04 * (TAU * 8_317.0 * time).sin()
                + 0.015 * random
        })
        .collect()
}

fn current_ffmpeg_filter(effects: SegmentEffects) -> String {
    let volume = db_to_gain(effects.volume_db);
    format!(
        "volume={volume:.6},bass=g={:.3}:f=250:t=s:w=1,equalizer=g={:.3}:f=1000:t=q:w=1,treble=g={:.3}:f=3000:t=s:w=1",
        effects.bass_db, effects.mid_db, effects.treble_db,
    )
}

fn render_with_ffmpeg(input: &[f32], filter: &str) -> Result<Vec<f32>, Box<dyn Error>> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "f32le",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-i",
            "pipe:0",
            "-af",
            filter,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("FFmpeg stdin is unavailable")?;
    let input_bytes: Vec<u8> = input
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    // Drain FFmpeg's stdout concurrently with writing stdin. Sequentially
    // filling both OS pipes deadlocks once a fixture exceeds the pipe buffer.
    let writer = std::thread::spawn(move || stdin.write_all(&input_bytes));
    let output = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| "FFmpeg input writer panicked")??;
    if !output.status.success() {
        return Err(format!("FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    if !output.stdout.len().is_multiple_of(size_of::<f32>()) {
        return Err("FFmpeg returned a partial f32 sample".into());
    }
    Ok(output
        .stdout
        .chunks_exact(size_of::<f32>())
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
        .collect())
}

#[derive(Debug)]
struct Measurement {
    residual_dbfs: f64,
    relative_residual_db: f64,
    max_abs: f32,
    correlation: f64,
}

fn compare(native: &[f32], reference: &[f32]) -> Measurement {
    let mut signal_squared = 0.0_f64;
    let mut residual_squared = 0.0_f64;
    let mut dot = 0.0_f64;
    let mut native_squared = 0.0_f64;
    let mut reference_squared = 0.0_f64;
    let mut max_abs = 0.0_f32;
    for (&actual, &expected) in native.iter().zip(reference) {
        let actual = f64::from(actual);
        let expected = f64::from(expected);
        let residual = actual - expected;
        signal_squared += expected * expected;
        residual_squared += residual * residual;
        dot += actual * expected;
        native_squared += actual * actual;
        reference_squared += expected * expected;
        max_abs = max_abs.max(residual.abs() as f32);
    }
    let length = native.len() as f64;
    let residual_rms = (residual_squared / length).sqrt();
    let signal_rms = (signal_squared / length).sqrt();
    Measurement {
        residual_dbfs: 20.0 * residual_rms.max(f64::MIN_POSITIVE).log10(),
        relative_residual_db: 20.0
            * (residual_rms / signal_rms.max(f64::MIN_POSITIVE))
                .max(f64::MIN_POSITIVE)
                .log10(),
        max_abs,
        correlation: dot / (native_squared * reference_squared).sqrt(),
    }
}

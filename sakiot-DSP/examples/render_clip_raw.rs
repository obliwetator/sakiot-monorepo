use sakiot_dsp::{SegmentEffects, render_clip_interleaved};
use std::error::Error;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !(5..=6).contains(&arguments.len()) {
        return Err(
            "usage: render_clip_raw SAMPLE_RATE CHANNELS PITCH_CENTS RATE REVERSE [TAIL_SECONDS]"
                .into(),
        );
    }
    let sample_rate = arguments[0].parse::<f64>()?;
    let channels = arguments[1].parse::<usize>()?;
    let effects = SegmentEffects {
        pitch_cents: arguments[2].parse()?,
        rate: arguments[3].parse()?,
        reverse: arguments[4].parse()?,
        tail_seconds: arguments
            .get(5)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or_default(),
        ..SegmentEffects::default()
    };
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    if !bytes.len().is_multiple_of(size_of::<f32>()) {
        return Err("stdin is not complete little-endian f32 PCM".into());
    }
    let samples: Vec<f32> = bytes
        .chunks_exact(size_of::<f32>())
        .map(|sample| f32::from_le_bytes(sample.try_into().expect("four-byte chunk")))
        .collect();
    let rendered = render_clip_interleaved(&samples, sample_rate, channels, effects)?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for sample in rendered {
        output.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

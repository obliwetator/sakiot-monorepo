use sakiot_dsp::{
    IncrementalRenderer, SegmentEffects, render_clip_interleaved, reverse_interleaved_frames,
};
use std::error::Error;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !(5..=8).contains(&arguments.len()) {
        return Err(
            "usage: render_clip_raw SAMPLE_RATE CHANNELS PITCH_CENTS RATE REVERSE [TAIL_SECONDS] [incremental|reference] [effects]"
                .into(),
        );
    }
    let sample_rate = arguments[0].parse::<f64>()?;
    let channels = arguments[1].parse::<usize>()?;
    let mut effects = SegmentEffects {
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
    if arguments.get(7).is_some_and(|arg| arg == "effects") {
        effects.distortion_wet = 0.2;
        effects.delay_seconds = 0.01;
        effects.delay_wet = 0.3;
        effects.compressor_enabled = true;
        effects.chorus_enabled = true;
        effects.reverb_enabled = true;
        effects.reverb_decay_seconds = 0.02;
        effects.reverb_wet = 0.2;
    }
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    if !bytes.len().is_multiple_of(size_of::<f32>()) {
        return Err("stdin is not complete little-endian f32 PCM".into());
    }
    let mut samples: Vec<f32> = bytes
        .chunks_exact(size_of::<f32>())
        .map(|sample| f32::from_le_bytes(sample.try_into().expect("four-byte chunk")))
        .collect();
    let rendered = if arguments.get(6).is_some_and(|arg| arg == "incremental") {
        if effects.reverse {
            reverse_interleaved_frames(&mut samples, channels);
            effects.reverse = false;
        }
        let mut renderer =
            IncrementalRenderer::new(sample_rate, channels, samples.len() / channels, effects)?;
        let mut output = Vec::new();
        let mut sink = |block: &[f32]| {
            output.extend_from_slice(block);
            Ok::<_, std::convert::Infallible>(())
        };
        for block in samples.chunks(137 * channels) {
            renderer.push(block, &mut sink)?;
        }
        renderer.finish(&mut sink)?;
        output
    } else {
        render_clip_interleaved(&samples, sample_rate, channels, effects)?
    };
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for sample in rendered {
        output.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

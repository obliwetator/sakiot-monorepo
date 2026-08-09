use sakiot_dsp::{SegmentEffects, SegmentProcessor};
use std::error::Error;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() != 29 {
        return Err(
            "usage: process_raw SAMPLE_RATE CHANNELS VOLUME_DB BASS_DB MID_DB TREBLE_DB DISTORTION_AMOUNT DISTORTION_WET DELAY_SECONDS DELAY_FEEDBACK DELAY_WET COMPRESSOR_ENABLED COMPRESSOR_THRESHOLD_DB COMPRESSOR_KNEE_DB COMPRESSOR_RATIO COMPRESSOR_ATTACK_SECONDS COMPRESSOR_RELEASE_SECONDS CHORUS_ENABLED CHORUS_FREQUENCY_HZ CHORUS_DELAY_MS CHORUS_DEPTH CHORUS_SPREAD_DEGREES CHORUS_FEEDBACK CHORUS_WET REVERB_ENABLED REVERB_DECAY_SECONDS REVERB_PRE_DELAY_SECONDS REVERB_WET REVERB_SEED".into(),
        );
    }
    let sample_rate = arguments[0].parse::<f64>()?;
    let channels = arguments[1].parse::<usize>()?;
    let effects = SegmentEffects {
        volume_db: arguments[2].parse()?,
        bass_db: arguments[3].parse()?,
        mid_db: arguments[4].parse()?,
        treble_db: arguments[5].parse()?,
        distortion_amount: arguments[6].parse()?,
        distortion_wet: arguments[7].parse()?,
        delay_seconds: arguments[8].parse()?,
        delay_feedback: arguments[9].parse()?,
        delay_wet: arguments[10].parse()?,
        compressor_enabled: arguments[11].parse()?,
        compressor_threshold_db: arguments[12].parse()?,
        compressor_knee_db: arguments[13].parse()?,
        compressor_ratio: arguments[14].parse()?,
        compressor_attack_seconds: arguments[15].parse()?,
        compressor_release_seconds: arguments[16].parse()?,
        chorus_enabled: arguments[17].parse()?,
        chorus_frequency_hz: arguments[18].parse()?,
        chorus_delay_ms: arguments[19].parse()?,
        chorus_depth: arguments[20].parse()?,
        chorus_spread_degrees: arguments[21].parse()?,
        chorus_feedback: arguments[22].parse()?,
        chorus_wet: arguments[23].parse()?,
        reverb_enabled: arguments[24].parse()?,
        reverb_decay_seconds: arguments[25].parse()?,
        reverb_pre_delay_seconds: arguments[26].parse()?,
        reverb_wet: arguments[27].parse()?,
        reverb_seed: arguments[28].parse()?,
        ..SegmentEffects::default()
    };
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    if !bytes.len().is_multiple_of(size_of::<f32>()) {
        return Err("stdin is not complete little-endian f32 PCM".into());
    }
    let mut samples: Vec<f32> = bytes
        .chunks_exact(size_of::<f32>())
        .map(|sample| f32::from_le_bytes(sample.try_into().expect("four-byte chunk")))
        .collect();
    let mut processor = SegmentProcessor::new(sample_rate, channels, effects)?;
    processor.process_interleaved(&mut samples)?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for sample in samples {
        output.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}
